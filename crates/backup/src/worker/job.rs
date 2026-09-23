//! 独立 BackupWorker：只消费封存票据所指的只读传输源，独占追加凭据。
//!
//! Intent 先持久，再动真实副作用；Outcome 在副作用之后记录。
//! 远端已确认而 Outcome 审计失败时保留真实状态与固定补记，不推进成功指针。
//!
//! 主流程（`run` 子模块）与补记、核对（`followup` 子模块）是同一 `BackupWorker`
//! 按职责分文件的 impl；失败模式（`error` 子模块）仍经本模块原路径导出，
//! 审计载荷在 `payload` 子模块。

mod error;
mod followup;
mod payload;
mod run;

pub use error::{AuditError2, WorkerError};

use super::recovery_point::BackupTargetKind;
use super::restic::{
    CredentialHolder, RepositoryRef, ResticError, ResticRunner, SnapshotRef, UnverifiedReason,
};
use super::state::{FailureReason, JobKey, JobRecord, RemoteVerification, UnknownStage};
use crate::store::model::{BackupStateStore, PersistedState, StoreError};
use std::num::NonZeroU64;
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditLedger, AuditPhase, SanitizedPayload};
use studiolo_core::checkpoint::CheckpointId;

/// 备份封存沿用既有 L2 动作名。
pub const BACKUP_AUDIT_ACTION: &str = "backup.snapshot";

/// 按作业键核对远端对象的 Interface；没有真实仓库时返回明确未知。
pub trait RemoteProbe {
    /// 沿原作业键定位远端对象；不存在返回 `None`。
    fn locate(&self, key: &JobKey) -> Result<Option<SnapshotRef>, ResticError>;

    /// 校验远端对象完整性、校验和与成对 checkpoint 引用。
    fn verify(
        &self,
        snapshot: &SnapshotRef,
        checkpoint_id: &CheckpointId,
    ) -> Result<RemoteVerification, ResticError>;
}

/// 一次作业的对外结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobOutcome {
    /// 已成功且存在可恢复点。
    Succeeded,
    /// 远端已确认但 Outcome 审计失败，等待固定补记。
    AuditPending,
    /// 明确失败。
    Failed(FailureReason),
    /// 明确未验证；既不是成功也不是失败。
    Unverified(UnverifiedReason),
    /// 结果未知，需沿原作业键核对。
    ResultUnknown(UnknownStage),
}

/// 独立身份的 BackupWorker。
pub struct BackupWorker<'a> {
    ledger: &'a dyn AuditLedger,
    runner: &'a dyn ResticRunner,
    probe: &'a dyn RemoteProbe,
    store: &'a dyn BackupStateStore,
    repository: RepositoryRef,
    target: BackupTargetKind,
}

impl<'a> BackupWorker<'a> {
    /// 组装 Worker；持有者不是 BackupWorker 即拒绝。
    pub fn new(
        ledger: &'a dyn AuditLedger,
        runner: &'a dyn ResticRunner,
        probe: &'a dyn RemoteProbe,
        store: &'a dyn BackupStateStore,
        repository: RepositoryRef,
        holder: CredentialHolder,
        target: BackupTargetKind,
    ) -> Result<Self, WorkerError> {
        if holder != CredentialHolder::BackupWorker {
            return Err(WorkerError::CredentialHolderForbidden);
        }
        Ok(Self {
            ledger,
            runner,
            probe,
            store,
            repository,
            target,
        })
    }

    fn state(&self) -> Result<PersistedState, WorkerError> {
        let state = self
            .store
            .load_state()?
            .unwrap_or_else(PersistedState::initial);
        state.validate()?;
        Ok(state)
    }

    fn append(
        &self,
        key: &JobKey,
        phase: AuditPhase,
        payload: serde_json::Value,
    ) -> Result<(), WorkerError> {
        let event_id = AuditEventId::parse(payload::event_id_text(key))
            .map_err(|error| WorkerError::Audit(error.into()))?;
        let payload = SanitizedPayload::try_from(payload)
            .map_err(|error| WorkerError::Audit(error.into()))?;
        self.ledger
            .append(AuditEvent::new(
                event_id,
                phase,
                BACKUP_AUDIT_ACTION,
                payload,
            ))
            .map_err(|error| WorkerError::Audit(error.into()))?;
        Ok(())
    }

    /// 排队一次备份：先持久 Intent，再持久排队记录。审计失败即失败关闭。
    pub fn enqueue(&self, checkpoint_id: &CheckpointId) -> Result<JobKey, WorkerError> {
        let mut state = self.state()?;
        let sequence = NonZeroU64::new(state.next_sequence).ok_or(StoreError::CorruptStore)?;
        let key = JobKey {
            sequence,
            checkpoint_id: checkpoint_id.clone(),
        };
        self.append(&key, AuditPhase::Intent, payload::intent(&key, self.target))?;
        state.next_sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or(StoreError::CorruptStore)?;
        state.jobs.push(JobRecord::queued(key.clone()));
        self.store.save_state(&state)?;
        Ok(key)
    }

    fn with_job<T>(
        &self,
        key: &JobKey,
        change: impl FnOnce(&mut JobRecord) -> Result<T, WorkerError>,
    ) -> Result<(T, PersistedState), WorkerError> {
        let mut state = self.state()?;
        let job = state
            .jobs
            .iter_mut()
            .find(|job| job.key == *key)
            .ok_or(WorkerError::JobNotFound)?;
        let value = change(job)?;
        self.store.save_state(&state)?;
        Ok((value, state))
    }

    fn record_outcome(&self, key: &JobKey, payload: serde_json::Value) -> Result<(), WorkerError> {
        self.append(key, AuditPhase::Outcome, payload)
    }
}
