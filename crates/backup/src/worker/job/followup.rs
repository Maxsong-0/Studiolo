//! 主流程之后的补记、结果未知核对、seed 校验与恢复点显示状态。

use super::super::recovery_point::{RecoveryPointInputs, RecoveryPointStatus, RpoBudget};
use super::super::state::{JobEvent, JobKey, JobState, UnknownStage};
use super::{BackupWorker, JobOutcome, WorkerError, payload};
use crate::checkpoint::barrier::BarrierClock;

impl BackupWorker<'_> {
    /// 仅补记已持久的失败事实，不重新执行上传或封存。
    pub fn backfill_failure(&self, key: &JobKey) -> Result<JobOutcome, WorkerError> {
        let mut state = self.state()?;
        let pending = state
            .pending_failure
            .as_ref()
            .filter(|p| &p.key == key)
            .ok_or(WorkerError::EvidenceMismatch)?;
        let (outcome, event) = match pending.unverified {
            Some(reason) => (
                JobOutcome::Unverified(reason),
                payload::outcome_unverified(key, reason),
            ),
            None => (
                JobOutcome::Failed(pending.reason),
                payload::outcome_failed(key, pending.reason),
            ),
        };
        self.record_outcome(key, event)?;
        state.pending_failure = None;
        self.store.save_state(&state)?;
        Ok(outcome)
    }
    /// 崩溃重启后只把执行中的阶段标为未知；不自动重新运行任何副作用。
    pub fn recover_interrupted(&self) -> Result<(), WorkerError> {
        let mut state = self.state()?;
        for job in &mut state.jobs {
            let stage = match job.state {
                JobState::Sealing => Some(UnknownStage::Sealing),
                JobState::Uploading => Some(UnknownStage::Uploading),
                JobState::Verifying => Some(UnknownStage::Verifying),
                JobState::CommittingFinalManifest => Some(UnknownStage::FinalManifestAck),
                _ => None,
            };
            if let Some(stage) = stage {
                job.advance(JobEvent::ResultUnknown { stage })?;
            }
        }
        self.store.save_state(&state)?;
        Ok(())
    }

    /// 上传阶段中断只重新查询原快照；本方法绝不调用上传执行器。
    pub fn reconcile_upload(
        &self,
        key: &JobKey,
        source: &crate::checkpoint::staging::SealedCheckpoint,
        probe: &dyn super::RemoteProbe,
        finalizer: &dyn crate::worker::final_manifest::FinalManifestWriter,
    ) -> Result<JobOutcome, WorkerError> {
        self.with_job(key, |job| {
            if job.source.as_ref() != Some(source.reference())
                || job.checkpoint_at != Some(source.ticket().content_cutoff)
            {
                return Err(WorkerError::EvidenceMismatch);
            }
            job.advance(JobEvent::ResumeRemoteVerification)?;
            Ok(())
        })?;
        self.verify_and_commit(key, source, probe, finalizer)
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
        let receipt = state
            .last_final
            .as_ref()
            .ok_or(WorkerError::SeedNotVerifiable)?;
        let has_success = state
            .jobs
            .iter()
            .any(|job| &job.key == receipt.draft().key() && job.state == JobState::Succeeded);
        if !has_success
            || self.store.load_final_manifest()?.as_ref() != Some(receipt.draft().manifest())
        {
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
