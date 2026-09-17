//! 只能消费同一私有配置的封闭批准；当前请求的 Vault UUID 不是根绑定证明。
use super::*;
use crate::coordinator::{Control, Core, WatchHealth};
use std::sync::MutexGuard;
use studiolo_host_config::{ApprovedRenamePair, FileOperationPolicy};
use studiolo_vault_broker::FileOperationLimits;

impl Core {
    pub(in crate::coordinator) fn enable_approved_file_operations(
        &self,
        broker: &mut FsVaultBroker,
    ) -> Result<(), crate::coordinator::Failure> {
        use crate::coordinator::Failure;
        self.roots.revalidate().map_err(|_| Failure::Binding)?;
        broker
            .validate_root_identities(
                self.roots.vault_identity(),
                self.roots.writer_data_identity(),
            )
            .map_err(|_| Failure::Binding)?;
        let Some(policy) = self.roots.file_operation_policy() else {
            return Ok(());
        };
        if self.content_audit.is_none() || broker.vault_uuid() != policy.vault_uuid() {
            return Err(Failure::Binding);
        }
        policy.revalidate().map_err(|_| Failure::Binding)?;
        let ledger = self.content_audit.as_ref().ok_or(Failure::Writer)?;
        let records = ledger
            .query(&studiolo_audit_ledger::AuditFilter::default().with_limit(1))
            .map_err(|_| Failure::Writer)?;
        for record in records {
            record.verify_integrity().map_err(|_| Failure::Writer)?;
        }
        // 审计查询可能等待；不能用查询前的批准初始化持久存储。
        self.roots.revalidate().map_err(|_| Failure::Binding)?;
        policy.revalidate().map_err(|_| Failure::Binding)?;
        broker
            .validate_root_identities(
                self.roots.vault_identity(),
                self.roots.writer_data_identity(),
            )
            .map_err(|_| Failure::Binding)?;
        let limits = FileOperationLimits {
            effective_object_limit: policy.effective_object_limit(),
            max_tombstones: policy.max_tombstones(),
            retained_bytes_limit: policy.retained_bytes_limit(),
            minimum_free_bytes: policy.minimum_free_bytes(),
        };
        // 查询既有批准不重复初始化；待补审计不能被初始化所需的空闲门禁挡住。
        match broker
            .file_operation_limits()
            .map_err(|_| Failure::Writer)?
        {
            Some(actual) if actual == limits => {}
            Some(_) => return Err(Failure::Binding),
            None => broker
                .enable_file_operations(limits)
                .map_err(|_| Failure::Writer)?,
        }
        if let Some(rename) = policy.rename_policy() {
            self.roots.revalidate().map_err(|_| Failure::Binding)?;
            rename.revalidate().map_err(|_| Failure::Binding)?;
            broker
                .validate_root_identities(
                    self.roots.vault_identity(),
                    self.roots.writer_data_identity(),
                )
                .map_err(|_| Failure::Binding)?;
            // 已持久化的许可不重复授予，让原审计 outbox 在随后启动阶段闭合。
            if !broker.file_rename_enabled().map_err(|_| Failure::Writer)? {
                broker.enable_file_rename().map_err(|_| Failure::Writer)?;
            }
            rename.revalidate().map_err(|_| Failure::Binding)?;
        }
        self.roots.revalidate().map_err(|_| Failure::Binding)
    }
}

impl CanonicalWorkbench {
    pub(super) fn file_input_access(
        &self,
        broker: &FsVaultBroker,
        principal: &ContentPrincipal,
        input: &Input,
    ) -> Result<(&FileOperationPolicy, Option<ApprovedRenamePair>), Problem> {
        let policy = self.file_access(broker, principal, input.vault, Some(&input.path))?;
        let pair = if let Kind::Rename { to, .. } = &input.kind {
            Some(
                policy
                    .rename_policy()
                    .ok_or_else(problem::rename_disabled)?
                    .approve_pair(input.vault, &input.path, to)
                    .map_err(|_| Problem::forbidden())?,
            )
        } else {
            None
        };
        Ok((policy, pair))
    }

    pub(super) fn file_access(
        &self,
        broker: &FsVaultBroker,
        principal: &ContentPrincipal,
        vault: VaultUuid,
        path: Option<&VaultPath>,
    ) -> Result<&FileOperationPolicy, Problem> {
        principal
            .verify()
            .map_err(super::super::content::session_error)?;
        self.core
            .roots
            .revalidate()
            .map_err(|_| problem::unavailable())?;
        broker
            .validate_root_identities(
                self.core.roots.vault_identity(),
                self.core.roots.writer_data_identity(),
            )
            .map_err(|_| problem::unavailable())?;
        let policy = self
            .core
            .roots
            .file_operation_policy()
            .filter(|_| self.core.content_audit.is_some())
            .ok_or_else(problem::disabled)?;
        if broker.vault_uuid() != vault || policy.vault_uuid() != vault {
            return Err(Problem::forbidden());
        }
        policy.revalidate().map_err(|_| problem::unavailable())?;
        if let Some(path) = path
            && !policy
                .allows(vault, path)
                .map_err(|_| problem::unavailable())?
        {
            return Err(Problem::forbidden());
        }
        let control = self
            .core
            .control
            .lock()
            .map_err(|_| problem::unavailable())?;
        if control.closing {
            return Err(problem::unavailable());
        }
        Ok(policy)
    }

    pub(super) fn file_gate(
        &self,
        broker: &FsVaultBroker,
        principal: &ContentPrincipal,
        input: &Input,
        epoch: u64,
    ) -> Result<MutexGuard<'_, Control>, Problem> {
        let (policy, pair) = self.file_input_access(broker, principal, input)?;
        let control = self
            .core
            .control
            .lock()
            .map_err(|_| problem::unavailable())?;
        if control.epoch != epoch || control.closing || control.health != WatchHealth::Healthy {
            return Err(Problem::stale());
        }
        // 会话数据库可能等待；最后一次真实核验不能使原配置批准跨等待失效。
        principal
            .verify()
            .map_err(super::super::content::session_error)?;
        self.core
            .roots
            .revalidate()
            .map_err(|_| problem::unavailable())?;
        policy.revalidate().map_err(|_| problem::unavailable())?;
        if let Some(pair) = pair {
            pair.revalidate().map_err(|_| problem::unavailable())?;
        }
        broker
            .validate_root_identities(
                self.core.roots.vault_identity(),
                self.core.roots.writer_data_identity(),
            )
            .map_err(|_| problem::unavailable())?;
        Ok(control)
    }
}
