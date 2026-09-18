//! 独立 BackupWorker：只消费封存票据所指的只读传输源，独占追加凭据。
//!
//! Intent 先持久，再动真实副作用；Outcome 在副作用之后记录。
//! 远端已确认而 Outcome 审计失败时保留真实状态与固定补记，不推进成功指针。

use super::recovery_point::{
    BackupTargetKind, RecoveryPointInputs, RecoveryPointStamps, RecoveryPointStatus, RpoBudget,
};
use super::restic::{
    CredentialHolder, RepositoryRef, ResticCommand, ResticError, ResticIdentity, ResticInvocation,
    ResticOutcome, ResticRunner, SnapshotRef, UnverifiedReason,
};
use super::state::{
    FailureReason, JobEvent, JobKey, JobRecord, JobState, ReconciledOutcome, RemoteVerification,
    StateError, UnknownStage, UploadEvidence,
};
use crate::checkpoint::barrier::BarrierClock;
use crate::checkpoint::manifest::{CheckpointManifest, ManifestError, ManifestInputs};
use crate::checkpoint::types::SealTicket;
use crate::store::model::{BackupStateStore, PersistedState, StoreError};
use std::num::NonZeroU64;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditLedger, AuditPhase, SanitizedPayload,
};
use studiolo_core::checkpoint::CheckpointId;

mod payload;

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

/// BackupWorker 的失败模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerError {
    /// 追加凭据被 BackupWorker 之外的组件持有。
    CredentialHolderForbidden,
    /// 审计写入失败；Intent 阶段失败关闭。
    Audit(AuditError2),
    /// 持久存储失败。
    Store(StoreError),
    /// 状态机拒绝。
    State(StateError),
    /// 清单组装失败。
    Manifest(ManifestError),
    /// restic 参数或身份不合法。
    Restic(ResticError),
    /// 作业不存在。
    JobNotFound,
    /// 尚无成功作业与已提交清单，不能标记 seed 已校验。
    SeedNotVerifiable,
}

/// 审计失败的可复制摘要；`AuditError` 本身不是 `Copy`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditError2 {
    /// 审计存储不可用或写入被拒绝。
    AppendFailed,
    /// 同一事件键被用于语义不同的内容。
    IdempotencyConflict,
}

impl From<AuditError> for AuditError2 {
    fn from(error: AuditError) -> Self {
        match error {
            AuditError::IdempotencyConflict { .. } => Self::IdempotencyConflict,
            _ => Self::AppendFailed,
        }
    }
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CredentialHolderForbidden => {
                formatter.write_str("追加凭据只能由 BackupWorker 持有")
            }
            Self::Audit(error) => write!(formatter, "备份审计失败：{error:?}"),
            Self::Store(error) => write!(formatter, "备份状态存储失败：{error}"),
            Self::State(error) => write!(formatter, "备份状态机拒绝：{error}"),
            Self::Manifest(error) => write!(formatter, "清单组装失败：{error}"),
            Self::Restic(error) => write!(formatter, "restic 调用不合法：{error}"),
            Self::JobNotFound => formatter.write_str("作业不存在"),
            Self::SeedNotVerifiable => formatter.write_str("尚无成功作业与已提交清单"),
        }
    }
}

impl std::error::Error for WorkerError {}

impl From<StoreError> for WorkerError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<StateError> for WorkerError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

impl From<ManifestError> for WorkerError {
    fn from(error: ManifestError) -> Self {
        Self::Manifest(error)
    }
}

impl From<ResticError> for WorkerError {
    fn from(error: ResticError) -> Self {
        Self::Restic(error)
    }
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

    /// 跑完封存后的上传、校验与最终清单提交。
    pub fn run<C: BarrierClock>(
        &self,
        key: &JobKey,
        ticket: &SealTicket,
        inputs: ManifestInputs,
        clock: &C,
    ) -> Result<JobOutcome, WorkerError> {
        self.with_job(key, |job| {
            job.advance(JobEvent::StartSealing {
                checkpoint_at: ticket.content_cutoff,
            })?;
            job.advance(JobEvent::SealCompleted)?;
            Ok(())
        })?;

        let tag = super::restic::JobTag::parse(format!("job-{}", key.sequence))?;
        let backup = ResticCommand::Backup {
            tag,
            source_root: ticket.transfer_source.root.clone(),
        };
        let invocation = ResticInvocation::plan(
            self.repository.clone(),
            &backup,
            CredentialHolder::BackupWorker,
            ResticIdentity::Append,
        )?;
        let outcome = self.runner.run(&invocation).map_err(WorkerError::Restic)?;
        let evidence = match outcome {
            ResticOutcome::Unverified { reason } => {
                self.finish_failed(key, FailureReason::TargetUnverified, Some(reason))?;
                return Ok(JobOutcome::Unverified(reason));
            }
            ResticOutcome::Exited {
                status_zero,
                reported_objects,
            } => UploadEvidence {
                exit_status_zero: status_zero,
                reported_objects,
            },
        };
        let (state, _) = self.with_job(key, |job| {
            job.advance(JobEvent::UploadExited { evidence })?;
            Ok(job.state)
        })?;
        if let JobState::Failed { reason } = state {
            self.record_outcome(key, payload::outcome_failed(key, reason))?;
            return Ok(JobOutcome::Failed(reason));
        }

        // 上传退出 0 只是中间证据：必须沿原作业键在远端真的找到对象。
        let Some(snapshot) = self.probe.locate(key).map_err(WorkerError::Restic)? else {
            self.finish_failed(key, FailureReason::RemoteVerificationFailed, None)?;
            return Ok(JobOutcome::Failed(FailureReason::RemoteVerificationFailed));
        };
        let verification = self
            .probe
            .verify(&snapshot, &key.checkpoint_id)
            .map_err(WorkerError::Restic)?;
        let (state, _) = self.with_job(key, |job| {
            job.advance(JobEvent::RemoteVerified { verification })?;
            Ok(job.state)
        })?;
        if let JobState::Failed { reason } = state {
            self.record_outcome(key, payload::outcome_failed(key, reason))?;
            return Ok(JobOutcome::Failed(reason));
        }

        let manifest = CheckpointManifest::build(ticket, inputs)?;
        if let Err(error) = self.store.commit_final_manifest(&manifest) {
            let reason = match error {
                StoreError::OutOfSpace => FailureReason::OutOfSpace,
                _ => FailureReason::FinalManifestCommitFailed,
            };
            self.finish_failed(key, reason, None)?;
            return Ok(JobOutcome::Failed(reason));
        }

        let finalized_at = clock.now();
        let audited = self
            .record_outcome(key, payload::outcome_success(key, &verification))
            .is_ok();
        let (state, mut persisted) = self.with_job(key, |job| {
            job.advance(JobEvent::FinalManifestCommitted {
                finalized_at,
                outcome_audit_succeeded: audited,
            })?;
            Ok(job.state)
        })?;
        if state.advances_success_pointer() {
            persisted.last_scheduled = Some(RecoveryPointStamps {
                checkpoint_at: ticket.content_cutoff,
                remote_verified_at: verification.verified_at,
                finalized_at,
            });
            self.store.save_state(&persisted)?;
            return Ok(JobOutcome::Succeeded);
        }
        // 远端确认为真实状态，成功指针不前移，等待固定补记。
        Ok(JobOutcome::AuditPending)
    }

    fn finish_failed(
        &self,
        key: &JobKey,
        reason: FailureReason,
        unverified: Option<UnverifiedReason>,
    ) -> Result<(), WorkerError> {
        self.with_job(key, |job| {
            job.advance(JobEvent::Failed { reason })?;
            Ok(())
        })?;
        let payload = match unverified {
            Some(reason) => payload::outcome_unverified(key, reason),
            None => payload::outcome_failed(key, reason),
        };
        self.record_outcome(key, payload)
    }

    fn record_outcome(&self, key: &JobKey, payload: serde_json::Value) -> Result<(), WorkerError> {
        self.append(key, AuditPhase::Outcome, payload)
    }

    /// `AuditPending` 的固定补记；补记成功才推进成功指针。
    pub fn backfill_outcome<C: BarrierClock>(
        &self,
        key: &JobKey,
        clock: &C,
    ) -> Result<JobOutcome, WorkerError> {
        let state = self.state()?;
        let job = state
            .jobs
            .iter()
            .find(|job| job.key == *key)
            .ok_or(WorkerError::JobNotFound)?;
        let JobState::AuditPending { verification } = job.state else {
            return Err(WorkerError::State(StateError::JobKeyMismatch));
        };
        let checkpoint_at = job.checkpoint_at.ok_or(StoreError::CorruptStore)?;
        let finalized_at = job.finalized_at.unwrap_or_else(|| clock.now());
        self.record_outcome(key, payload::outcome_success(key, &verification))?;
        let (_, mut persisted) = self.with_job(key, |job| {
            job.advance(JobEvent::OutcomeBackfilled)?;
            Ok(())
        })?;
        persisted.last_scheduled = Some(RecoveryPointStamps {
            checkpoint_at,
            remote_verified_at: verification.verified_at,
            finalized_at,
        });
        self.store.save_state(&persisted)?;
        Ok(JobOutcome::Succeeded)
    }

    /// 沿原作业键核对结果未知的作业。
    pub fn reconcile_unknown(&self, key: &JobKey) -> Result<JobState, WorkerError> {
        let located = self.probe.locate(key).map_err(WorkerError::Restic)?;
        let outcome = match located {
            None => ReconciledOutcome::RemoteAbsent,
            Some(snapshot) => ReconciledOutcome::RemoteVerified {
                verification: self
                    .probe
                    .verify(&snapshot, &key.checkpoint_id)
                    .map_err(WorkerError::Restic)?,
            },
        };
        let (state, _) = self.with_job(key, |job| {
            job.reconcile_unknown(key, outcome)?;
            Ok(job.state)
        })?;
        Ok(state)
    }

    /// 记录一次阶段性结果未知。
    pub fn mark_unknown(&self, key: &JobKey, stage: UnknownStage) -> Result<(), WorkerError> {
        self.with_job(key, |job| {
            job.advance(JobEvent::ResultUnknown { stage })?;
            Ok(())
        })?;
        Ok(())
    }

    /// 首次完整 seed 校验：必须已有成功作业且清单已提交。
    pub fn mark_seed_verified(&self) -> Result<(), WorkerError> {
        let mut state = self.state()?;
        let has_success = state
            .jobs
            .iter()
            .any(|job| job.state.advances_success_pointer());
        let has_manifest = self.store.load_final_manifest()?.is_some();
        if !has_success || !has_manifest {
            return Err(WorkerError::SeedNotVerifiable);
        }
        state.seed_verified = true;
        self.store.save_state(&state)?;
        Ok(())
    }

    /// 计算恢复点显示状态。
    pub fn recovery_point_status<C: BarrierClock>(
        &self,
        clock: Option<&C>,
        budget: RpoBudget,
        immutability_proven: bool,
    ) -> Result<RecoveryPointStatus, WorkerError> {
        let state = self.state()?;
        Ok(RecoveryPointStatus::evaluate(RecoveryPointInputs {
            seed_verified: state.seed_verified,
            scheduled: state.last_scheduled,
            daily_immutable: state.last_daily_immutable,
            target: self.target,
            immutability_proven,
            now: clock.map(BarrierClock::now),
            budget,
        }))
    }
}
