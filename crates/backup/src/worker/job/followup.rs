//! 主流程之后的补记、结果未知核对、seed 校验与恢复点显示状态。

use super::super::recovery_point::{
    RecoveryPointInputs, RecoveryPointStamps, RecoveryPointStatus, RpoBudget,
};
use super::super::state::{
    JobEvent, JobKey, JobState, ReconciledOutcome, StateError, UnknownStage,
};
use super::{BackupWorker, JobOutcome, WorkerError, payload};
use crate::checkpoint::barrier::BarrierClock;
use crate::store::model::StoreError;

impl BackupWorker<'_> {
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
