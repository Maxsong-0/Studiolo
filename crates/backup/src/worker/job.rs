//! 独立 BackupWorker：只消费封存票据所指的只读传输源，独占追加凭据。
//!
//! Intent 先持久，再动真实副作用；Outcome 在副作用之后记录。
//! 远端已确认而 Outcome 审计失败时保留真实状态与固定补记，不推进成功指针。
//!
//! 主流程（`run` 子模块）与补记、核对（`followup` 子模块）是同一 `BackupWorker`
//! 按职责分文件的 impl；失败模式（`error` 子模块）仍经本模块原路径导出，
//! 审计载荷在 `payload` 子模块。

mod accept;
mod error;
mod finalize;
mod followup;
pub(super) mod payload;
mod run;

pub use accept::JobPosition;
pub use error::{AuditError2, WorkerError};

use super::recovery_point::BackupTargetKind;
use super::restic::{
    CredentialHolder, RepositoryRef, ResticError, ResticRunner, SnapshotRef, UnverifiedReason,
};
use super::state::{FailureReason, JobKey, JobRecord, JobState, RemoteVerification, UnknownStage};
use crate::store::model::{BackupStateStore, MAX_JOBS, MAX_QUEUED, PersistedState, StoreError};
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

/// 上传与数据复验端口由启动方显式装配；最终记录的读写能力独立传入。
pub struct DataRemote<'a> {
    /// 只接受封闭上传计划的执行器。
    pub runner: &'a dyn ResticRunner,
    /// 沿原作业核对完整数据快照。
    pub probe: &'a dyn RemoteProbe,
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
    store: &'a dyn BackupStateStore,
    repository: RepositoryRef,
    target: BackupTargetKind,
}

/// 排队与协调器核验共用唯一 Intent 映射，避免两端事件内容漂移。
pub fn backup_intent(key: &JobKey, target: BackupTargetKind) -> Result<AuditEvent, WorkerError> {
    let id = AuditEventId::parse(payload::event_id_text(key))
        .map_err(|error| WorkerError::Audit(error.into()))?;
    let payload = SanitizedPayload::try_from(payload::intent(key, target))
        .map_err(|error| WorkerError::Audit(error.into()))?;
    Ok(AuditEvent::new(
        id,
        AuditPhase::Intent,
        BACKUP_AUDIT_ACTION,
        payload,
    ))
}

/// 从账本里的一条备份作业事件读回原作业键；事件键和载荷对不上就不算。
///
/// 准入事件用的是另一种事件键，这里一律读不出来，要用 `AdmissionFact::from_receipt`。
pub fn backup_event_key(event: &AuditEvent) -> Option<JobKey> {
    if event.action() != BACKUP_AUDIT_ACTION {
        return None;
    }
    let value = event.payload().as_value();
    let sequence = NonZeroU64::new(value.get("job_sequence")?.as_u64()?)?;
    let checkpoint_id = CheckpointId::parse(value.get("checkpoint_id")?.as_str()?).ok()?;
    let key = JobKey {
        sequence,
        checkpoint_id,
    };
    (event.event_id().as_str() == payload::event_id_text(&key)).then_some(key)
}

/// 生产者在封存没做成时补的固定失败结果；和 worker 写的是同一个形状。
pub fn backup_failure(key: &JobKey, reason: FailureReason) -> Result<AuditEvent, WorkerError> {
    let id = AuditEventId::parse(payload::event_id_text(key))
        .map_err(|error| WorkerError::Audit(error.into()))?;
    let payload = SanitizedPayload::try_from(payload::outcome_failed(key, reason))
        .map_err(|error| WorkerError::Audit(error.into()))?;
    Ok(AuditEvent::new(
        id,
        AuditPhase::Outcome,
        BACKUP_AUDIT_ACTION,
        payload,
    ))
}

impl<'a> BackupWorker<'a> {
    /// 组装 Worker；持有者不是 BackupWorker 即拒绝。
    pub fn new(
        ledger: &'a dyn AuditLedger,
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
        let expected = AuditEvent::new(event_id, phase, BACKUP_AUDIT_ACTION, payload);
        let record = self
            .ledger
            .append(expected.clone())
            .map_err(|error| WorkerError::Audit(error.into()))?;
        record
            .verify_integrity()
            .map_err(|error| WorkerError::Audit(error.into()))?;
        if record.event() != &expected {
            return Err(WorkerError::Audit(
                studiolo_audit_ledger::AuditError::CorruptStore.into(),
            ));
        }
        Ok(())
    }

    /// 排队一次备份：先持久 Intent，再持久排队记录。审计失败即失败关闭。
    pub fn enqueue(&self, checkpoint_id: &CheckpointId) -> Result<JobKey, WorkerError> {
        let mut state = self.state()?;
        make_room(&mut state)?;
        let next_sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or(StoreError::LimitExceeded)?;
        let sequence = NonZeroU64::new(state.next_sequence).ok_or(StoreError::CorruptStore)?;
        let key = JobKey {
            sequence,
            checkpoint_id: checkpoint_id.clone(),
        };
        let expected = backup_intent(&key, self.target)?;
        let record = self
            .ledger
            .append(expected.clone())
            .map_err(|error| WorkerError::Audit(error.into()))?;
        record
            .verify_integrity()
            .map_err(|error| WorkerError::Audit(error.into()))?;
        if record.event() != &expected {
            return Err(WorkerError::Audit(
                studiolo_audit_ledger::AuditError::CorruptStore.into(),
            ));
        }
        state.next_sequence = next_sequence;
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

/// 排队前腾位置：等待队列有上限；记录满了就回收最旧的终态，保留最近成功和待补记的那条。
fn make_room(state: &mut PersistedState) -> Result<(), WorkerError> {
    if state
        .jobs
        .iter()
        .filter(|j| j.state == JobState::Queued)
        .count()
        >= MAX_QUEUED
    {
        return Err(StoreError::LimitExceeded.into());
    }
    if state.jobs.len() == MAX_JOBS {
        let last = state.last_final.as_ref().map(|r| r.draft().key());
        let oldest = state
            .jobs
            .iter()
            .position(|j| {
                j.state.is_terminal()
                    && Some(&j.key) != last
                    && state
                        .pending_failure
                        .as_ref()
                        .is_none_or(|pending| pending.key != j.key)
            })
            .ok_or(StoreError::LimitExceeded)?;
        state.jobs.remove(oldest);
    }
    Ok(())
}
