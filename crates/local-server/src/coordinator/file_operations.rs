//! 默认关闭的人侧整文件 owner；没有任意 Broker 回调或另一条写者队列。
mod access;
mod audit;
mod execution;
mod rename;
use crate::file_operations::{Input, Kind, problem};
use crate::{CanonicalWorkbench, auth_http::ContentPrincipal, editing::Problem};
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_vault_broker::{
    FileOperationMetadata, FileOperationReceipt, FileOperationRequest, FileOperationScopeHash,
    FsVaultBroker, MutationActor, TombstonePage, VaultReader,
};

impl CanonicalWorkbench {
    pub(crate) fn file_operation_capability(
        &self,
        principal: ContentPrincipal,
    ) -> Result<(VaultUuid, bool), Problem> {
        let owner = self.core.owner.lock().map_err(|_| problem::unavailable())?;
        let broker = owner.as_ref().ok_or_else(problem::unavailable)?;
        let vault = broker.vault_uuid();
        let policy = self.file_access(broker, &principal, vault, None)?;
        let enabled = if let Some(rename) = policy.rename_policy() {
            rename.revalidate().map_err(|_| problem::unavailable())?;
            broker
                .file_rename_enabled()
                .map_err(|_| problem::unavailable())?
        } else {
            false
        };
        Ok((vault, enabled))
    }

    pub(crate) fn tombstones(
        &self,
        principal: ContentPrincipal,
        vault: VaultUuid,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<TombstonePage, Problem> {
        let owner = self.core.owner.lock().map_err(|_| problem::unavailable())?;
        let broker = owner.as_ref().ok_or_else(problem::unavailable)?;
        let policy = self.file_access(broker, &principal, vault, None)?;
        // 仅一次有界页；Broker当前底层固定最多4096条元数据全读排序，limit不是物理扫描上限。
        let page = broker
            .list_tombstones(limit, cursor)
            .map_err(problem::query)?;
        let mut entries = Vec::with_capacity(page.entries.len());
        for entry in page.entries {
            if policy
                .allows(vault, &entry.path)
                .map_err(|_| problem::unavailable())?
            {
                entries.push(entry);
            }
        }
        self.file_access(broker, &principal, vault, None)?;
        Ok(TombstonePage {
            entries,
            next_cursor: page.next_cursor,
        })
    }

    pub(crate) fn human_file_operation(
        &self,
        principal: ContentPrincipal,
        input: Input,
    ) -> Result<FileOperationReceipt, Problem> {
        let mut owner = self.core.owner.lock().map_err(|_| problem::unavailable())?;
        let broker = owner.as_mut().ok_or_else(problem::unavailable)?;
        let (policy, _) = self.file_input_access(broker, &principal, &input)?;
        let (issuer, subject) = principal.actor();
        let domain = match input.kind {
            Kind::Delete(_) => "human.file.delete.v1",
            Kind::Restore(_) => "human.file.restore-tombstone.v1",
            Kind::Rename { .. } => "human.file.rename.v1",
        };
        let bytes = serde_json::to_vec(&(domain, issuer, subject, input.vault, &input.key))
            .map_err(|_| problem::unavailable())?;
        let key = format!("human-file-{}", crate::editing::digest(&bytes));
        let request = match &input.kind {
            Kind::Rename { expected, to } => FileOperationRequest::Rename {
                from: input.path.clone(),
                to: to.clone(),
                expected: *expected,
                idempotency_key: key,
            },
            Kind::Delete(expected) => FileOperationRequest::Delete {
                path: input.path.clone(),
                expected: *expected,
                idempotency_key: key,
            },
            Kind::Restore(id) => FileOperationRequest::RestoreTombstone {
                path: input.path.clone(),
                tombstone_id: id.clone(),
                idempotency_key: key,
            },
        };
        let metadata = FileOperationMetadata::new(
            MutationActor::Human {
                issuer: issuer.into(),
                subject: subject.into(),
            },
            FileOperationScopeHash::new(policy.scope_hash()).map_err(|_| problem::unavailable())?,
            policy.scope_epoch(),
        )
        .map_err(|_| problem::unavailable())?;
        // 原键查询先于freshness；真实持久旧回执不声称磁盘现在仍相同。
        if let Some(status) = broker
            .file_operation_replay(&request, &metadata)
            .map_err(problem::before_issue)?
        {
            let flushed = self.core.flush_file_operation_audit(broker);
            self.file_input_access(broker, &principal, &input)?;
            if flushed.is_err() {
                return Err(problem::unknown());
            }
            let status = broker
                .file_operation_replay(&request, &metadata)
                .map_err(|_| problem::unknown())?
                .unwrap_or(status);
            if !status.audit_pending
                && let Some(receipt) = status.receipt
            {
                return problem::receipt(receipt);
            }
            // 跨请求的Prepared/Issued不由当前内容或新快照猜测完成，也不重派Issued。
            return Err(problem::unknown());
        }
        if self.core.flush_file_operation_audit(broker).is_err() {
            return Err(problem::audit_unavailable());
        }
        let _ = self.core.flush_content_audit(broker);
        if broker
            .content_audit_status()
            .map_or(true, |status| status.restore_unconfirmed)
        {
            return Err(problem::audit_unavailable());
        }
        let epoch = self.content_epoch(broker)?;
        self.execute_file_operation(broker, &principal, &input, request, metadata, epoch)
    }
}
