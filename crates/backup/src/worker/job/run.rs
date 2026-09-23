//! 一次作业的主流程：封存后上传、远端校验、最终清单提交与 Outcome 审计；明确失败在这里收尾。

use super::super::recovery_point::RecoveryPointStamps;
use super::super::restic::{
    CredentialHolder, ResticCommand, ResticIdentity, ResticInvocation, ResticOutcome,
    UnverifiedReason,
};
use super::super::state::{FailureReason, JobEvent, JobKey, JobState, UploadEvidence};
use super::{BackupWorker, JobOutcome, WorkerError, payload};
use crate::checkpoint::barrier::BarrierClock;
use crate::checkpoint::manifest::{CheckpointManifest, ManifestInputs};
use crate::checkpoint::types::SealTicket;
use crate::store::model::StoreError;

impl BackupWorker<'_> {
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

        let tag = super::super::restic::JobTag::parse(format!("job-{}", key.sequence))?;
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
}
