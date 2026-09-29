//! 双向 Bridge 对 canonical 的唯一接缝（ADR-025 D8）：完整观察、有界读、条件写与条件删除；本文件是 `sync_plane` 触及 Broker 的唯一位置，住在 `coordinator/` 之下受同一写者边界守卫；
//! 每个方法持 owner 锁复验根身份，主体一律「外部：Obsidian」，幂等键由调用方按执行器原键给出；Broker 调用形态由守卫 `canonical_owner.rs` 逐条列名，这里只用那些形态。
mod errors;

use errors::{before_visible, install_error, not_attempted, observation_error, write_error};

use crate::{CanonicalWorkbench, LocalError};
use std::sync::MutexGuard;
use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{
    BlobIdentity, BlobSource, BrokerError, CanonicalRead, CanonicalSyncObservation,
    FileOperationMetadata, FileOperationReceipt, FileOperationRequest, FileOperationScopeHash,
    FsVaultBroker, Mutation, MutationActor, MutationKind, MutationMetadata, MutationReceipt,
    Precondition, StreamLimits, StreamMutation, SyncReadSelection,
};

/// 接缝的固定失败分类；不带路径或正文。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeSeamError {
    /// 还没碰 Broker 就失败——写者未接上／关停中／根复验失败／路径受保护／预算类，动作确定未安装。
    NotAttempted,
    /// 写者不可用且不能断言未安装（安装之后或文件操作准备之后的失败）。
    Unavailable,
    /// 观察不完整（诊断非空）或范围外路径。
    Incomplete,
    /// CAS 前置不满足；权威拒绝，可记 NoEffect。
    Rejected,
    /// 已发出安装但终态无法确认。
    Unknown,
    /// 审计账本不可用。
    Audit,
}
/// 条件写的目标前置：不存在（创建）或等于某 revision（替换）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgePrecondition {
    /// 目标必须不存在。
    MustNotExist,
    /// 目标当前必须是此 revision。
    Match(Revision),
}

impl CanonicalWorkbench {
    /// 持 owner 锁并复验根身份；关闭中、写者缺席或身份漂移一律 `NotAttempted`（尚未碰 Broker）。
    fn attach(&self) -> Result<MutexGuard<'_, Option<FsVaultBroker>>, BridgeSeamError> {
        if self.core.control.lock().map_err(not_attempted)?.closing {
            return Err(BridgeSeamError::NotAttempted);
        }
        let owner = self.core.owner.lock().map_err(not_attempted)?;
        let broker = owner.as_ref().ok_or(BridgeSeamError::NotAttempted)?;
        self.core.roots.revalidate().map_err(not_attempted)?;
        broker
            .validate_root_identities(
                self.core.roots.vault_identity(),
                self.core.roots.writer_data_identity(),
            )
            .map_err(not_attempted)?;
        Ok(owner)
    }
    /// canonical 已变：先标记视图过期，再请求一次后台刷新（失败只影响视图新鲜度）。
    fn canonical_changed(&self) {
        if let Ok(control) = self.core.control.lock() {
            self.core.mark_stale(&control, "changed");
        }
        let _ = self.core.request("changed");
    }
    /// 权威账本；没装配就是 `Audit`。
    fn ledger(&self) -> Result<&dyn studiolo_audit_ledger::AuditLedger, BridgeSeamError> {
        let ledger = self.core.content_audit.as_deref();
        ledger.ok_or(BridgeSeamError::Audit)
    }

    /// 完整观察批准范围（元数据调查 → 逐文件流式事实）；范围外或不完整返回 `Incomplete`。
    pub(crate) fn bridge_observe(
        &self,
        selection: &SyncReadSelection,
        required: &[VaultPath],
    ) -> Result<CanonicalSyncObservation, BridgeSeamError> {
        let mut owner = self.attach()?;
        let broker = owner.as_mut().ok_or(BridgeSeamError::NotAttempted)?;
        let survey = broker
            .begin_sync_observation(selection, required)
            .map_err(observation_error)?;
        broker
            .finish_sync_observation(survey, &[])
            .map_err(observation_error)
    }

    /// 有界读取单个 canonical 文件（≤ `max_bytes`）；不存在、超预算或不稳定返回 `Rejected`。
    pub(crate) fn bridge_read(
        &self,
        path: &VaultPath,
        max_bytes: u64,
    ) -> Result<CanonicalRead, BridgeSeamError> {
        self.read_file(path, max_bytes)
            .map_err(|error| match error {
                LocalError::Read(
                    BrokerError::NotFound(_)
                    | BrokerError::UnstableRead(_)
                    | BrokerError::ReadLimitExceeded { .. },
                ) => BridgeSeamError::Rejected,
                _ => BridgeSeamError::Unavailable,
            })
    }

    /// 整篇条件写（≤ 4 MiB）：先按原键查重放（有回执直接返回，绝不第二次安装），再创建或替换。
    pub(crate) fn bridge_write(
        &self,
        path: &VaultPath,
        content: Vec<u8>,
        precondition: BridgePrecondition,
        idempotency_key: &str,
    ) -> Result<MutationReceipt, BridgeSeamError> {
        let mut owner = self.attach()?;
        let broker = owner.as_mut().ok_or(BridgeSeamError::NotAttempted)?;
        let metadata = MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace)
            .map_err(not_attempted)?;
        let mutation = match precondition {
            BridgePrecondition::MustNotExist => {
                Mutation::create(path.clone(), content, idempotency_key)
            }
            BridgePrecondition::Match(expected) => {
                Mutation::replace(path.clone(), content, expected, idempotency_key)
            }
        };
        let _ = self.core.flush_content_audit(broker);
        if let Some(receipt) = broker
            .mutation_replay_recorded(&mutation, &metadata)
            .map_err(write_error)?
        {
            return Ok(receipt);
        }
        if broker
            .content_audit_status()
            .map_or(true, |status| status.restore_unconfirmed)
        {
            return Err(BridgeSeamError::Audit);
        }
        let result = broker.mutate_recorded(mutation, metadata);
        self.canonical_changed();
        let _ = self.core.flush_content_audit(broker);
        result.map_err(|error| install_error(broker, error))
    }

    /// 大对象条件写（> 4 MiB）：只读来源走流式准备／提交；准备后先把固定 Intent 追加进权威账本（按事件幂等），追加失败就取消票据。
    pub(crate) fn bridge_write_stream(
        &self,
        path: &VaultPath,
        source: BlobSource,
        content: BlobIdentity,
        precondition: BridgePrecondition,
        idempotency_key: &str,
    ) -> Result<MutationReceipt, BridgeSeamError> {
        let mut owner = self.attach()?;
        let broker = owner.as_mut().ok_or(BridgeSeamError::NotAttempted)?;
        let ledger = self.ledger()?;
        let metadata = MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace)
            .map_err(not_attempted)?;
        let limits = StreamLimits::new(content.bytes.max(1)).map_err(not_attempted)?;
        let request = StreamMutation {
            path: path.clone(),
            precondition: match precondition {
                BridgePrecondition::MustNotExist => Precondition::MustNotExist,
                BridgePrecondition::Match(expected) => Precondition::Match(expected),
            },
            idempotency_key: idempotency_key.to_owned(),
            content,
        };
        let _ = self.core.flush_content_audit(broker);
        if let Some(receipt) = broker
            .stream_mutation_replay(&request, &metadata)
            .map_err(write_error)?
        {
            return Ok(receipt);
        }
        let ticket = broker
            .prepare_stream_mutation(request, &source, limits, metadata)
            .map_err(write_error)?;
        if crate::editing::audit::append(ledger, ticket.intent()).is_err() {
            let _ = broker.cancel_stream_mutation(ticket);
            let _ = self.core.flush_content_audit(broker);
            return Err(BridgeSeamError::Audit);
        }
        let result = broker.commit_stream_mutation(ticket);
        self.canonical_changed();
        let _ = self.core.flush_content_audit(broker);
        result.map_err(|error| install_error(broker, error))
    }

    /// 条件删除走文件操作协议（保全进墓碑可恢复）：准备 → Intent 追加并确认 → 提交 → 冲刷 outbox；已有终态的原键在准备阶段就拿回固定票据。
    pub(crate) fn bridge_delete(
        &self,
        path: &VaultPath,
        expected: Revision,
        idempotency_key: &str,
        scope_hash: &str,
        scope_epoch: u64,
    ) -> Result<FileOperationReceipt, BridgeSeamError> {
        let mut owner = self.attach()?;
        let broker = owner.as_mut().ok_or(BridgeSeamError::NotAttempted)?;
        let ledger = self.ledger()?;
        let request = FileOperationRequest::Delete {
            path: path.clone(),
            expected,
            idempotency_key: idempotency_key.to_owned(),
        };
        let metadata = FileOperationScopeHash::new(scope_hash)
            .and_then(|hash| FileOperationMetadata::new(MutationActor::Obsidian, hash, scope_epoch))
            .map_err(not_attempted)?;
        if self.core.flush_file_operation_audit(broker).is_err() {
            return Err(BridgeSeamError::Audit);
        }
        let ticket = broker
            .prepare_file_operation(request, metadata)
            .map_err(write_error)?;
        let acknowledged = crate::file_operation_audit::append(ledger, ticket.intent())
            .map_err(|_| ())
            .and_then(|(offset, hash)| {
                self.core.roots.revalidate().map_err(|_| ())?;
                broker
                    .confirm_file_operation_intent(&ticket, offset, &hash)
                    .map_err(|_| ())
            });
        if acknowledged.is_err() {
            let _ = broker.cancel_file_operation(ticket);
            let _ = self.core.flush_file_operation_audit(broker);
            return Err(BridgeSeamError::Audit);
        }
        let result = broker.commit_file_operation(&ticket);
        if result.is_err() && before_visible(broker) {
            // 尚未进入可见执行：闭合票据不留半截；取消本身未经确认，仍按 `Unavailable` 交原键核对。
            let _ = broker.cancel_file_operation(ticket);
            let _ = self.core.flush_file_operation_audit(broker);
            return Err(BridgeSeamError::Unavailable);
        }
        drop(ticket);
        self.canonical_changed();
        let flushed = self.core.flush_file_operation_audit(broker);
        let receipt = result.map_err(|_| BridgeSeamError::Unknown)?;
        if flushed.map_or(true, |status| status.is_some()) {
            return Err(BridgeSeamError::Unknown);
        }
        Ok(receipt)
    }

    /// 冲刷内容与文件操作两条审计 outbox；任一失败返回 `Audit`。
    pub(crate) fn bridge_flush_audit(&self) -> Result<(), BridgeSeamError> {
        let mut owner = self.attach()?;
        let broker = owner.as_mut().ok_or(BridgeSeamError::NotAttempted)?;
        let content = self.core.flush_content_audit(broker).map(|_| ());
        let files = self.core.flush_file_operation_audit(broker).map(|_| ());
        content.and(files).map_err(|()| BridgeSeamError::Audit)
    }
}
