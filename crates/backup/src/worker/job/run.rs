//! 真实封存上传后，先持久最终记录草稿，再执行唯一一次远端提交。
use super::{BackupWorker, DataRemote, JobOutcome, WorkerError};
use crate::{
    checkpoint::staging::SealedCheckpoint,
    store::model::{PendingFailure, PendingFinal},
    worker::{
        final_manifest::{FinalManifestDraft, FinalManifestWriter},
        restic::{
            CredentialHolder, JobTag, ResticCommand, ResticIdentity, ResticInvocation,
            ResticOutcome, UnverifiedReason,
        },
        state::{FailureReason, JobEvent, JobKey, JobState, UnknownStage, UploadEvidence},
    },
};

impl BackupWorker<'_> {
    /// 只接收真实封存能力；调用方不能另传一份 manifest 或任意源路径。
    pub fn run(
        &self,
        key: &JobKey,
        source: &SealedCheckpoint,
        remote: DataRemote<'_>,
        finalizer: &dyn FinalManifestWriter,
    ) -> Result<JobOutcome, WorkerError> {
        if source.reference().checkpoint_id != key.checkpoint_id {
            return Err(WorkerError::EvidenceMismatch);
        }
        let mut state = self.state()?;
        if state.pending_failure.is_some()
            || state
                .jobs
                .iter()
                .any(|j| j.state != JobState::Queued && !j.state.is_terminal())
        {
            return Err(WorkerError::JobBusy);
        }
        let job = state
            .jobs
            .iter_mut()
            .find(|j| &j.key == key)
            .ok_or(WorkerError::JobNotFound)?;
        job.advance(JobEvent::StartSealing {
            checkpoint_at: source.ticket().content_cutoff,
        })?;
        job.source = Some(source.reference().clone());
        job.advance(JobEvent::SealCompleted)?;
        self.store.save_state(&state)?;

        let backup = ResticCommand::Backup {
            tag: JobTag::for_job(key)?,
            source_root: source.ticket().transfer_source.root.clone(),
        };
        let invocation = ResticInvocation::plan(
            self.repository.clone(),
            &backup,
            CredentialHolder::BackupWorker,
            ResticIdentity::Append,
        )?;
        let outcome = match remote.runner.run(&invocation) {
            Ok(outcome) => outcome,
            Err(_) => return self.unknown(key, UnknownStage::Uploading),
        };
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
        if !evidence.exit_status_zero {
            self.finish_failed(key, FailureReason::UploadRejected, None)?;
            return Ok(JobOutcome::Failed(FailureReason::UploadRejected));
        }
        self.with_job(key, |job| {
            job.advance(JobEvent::UploadExited { evidence })?;
            Ok(())
        })?;
        self.verify_and_commit(key, source, remote.probe, finalizer)
    }

    pub(super) fn verify_and_commit(
        &self,
        key: &JobKey,
        source: &SealedCheckpoint,
        probe: &dyn super::RemoteProbe,
        finalizer: &dyn FinalManifestWriter,
    ) -> Result<JobOutcome, WorkerError> {
        let snapshot = match probe.locate(key) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.finish_failed(key, FailureReason::RemoteVerificationFailed, None)?;
                return Ok(JobOutcome::Failed(FailureReason::RemoteVerificationFailed));
            }
            Err(_) => return self.unknown(key, UnknownStage::Verifying),
        };
        let verification = match probe.verify(&snapshot, &key.checkpoint_id) {
            Ok(verification) => verification,
            Err(_) => return self.unknown(key, UnknownStage::Verifying),
        };
        if !verification.is_recoverable() {
            self.finish_failed(key, FailureReason::RemoteVerificationFailed, None)?;
            return Ok(JobOutcome::Failed(FailureReason::RemoteVerificationFailed));
        }
        let draft = FinalManifestDraft::new(key.clone(), source, snapshot, verification)
            .map_err(|_| WorkerError::EvidenceMismatch)?;
        let mut state = self.state()?;
        let job = state
            .jobs
            .iter_mut()
            .find(|j| &j.key == key)
            .ok_or(WorkerError::JobNotFound)?;
        if job.source.as_ref() != Some(source.reference()) {
            return Err(WorkerError::EvidenceMismatch);
        }
        job.advance(JobEvent::RemoteVerified { verification })?;
        state.pending_final = Some(PendingFinal {
            draft: draft.clone(),
            receipt: None,
        });
        // 这一保存失败时绝不执行最终远端写入；重启后仍沿原作业核对。
        self.store.save_state(&state)?;
        match finalizer.commit_final(&draft) {
            Ok(receipt) if receipt.draft() == &draft => self.accept_receipt(key, receipt),
            Ok(_) | Err(_) => self.unknown(key, UnknownStage::FinalManifestAck),
        }
    }

    pub(super) fn unknown(
        &self,
        key: &JobKey,
        stage: UnknownStage,
    ) -> Result<JobOutcome, WorkerError> {
        self.mark_unknown(key, stage)?;
        Ok(JobOutcome::ResultUnknown(stage))
    }

    pub(super) fn finish_failed(
        &self,
        key: &JobKey,
        reason: FailureReason,
        unverified: Option<UnverifiedReason>,
    ) -> Result<(), WorkerError> {
        let mut state = self.state()?;
        let job = state
            .jobs
            .iter_mut()
            .find(|j| &j.key == key)
            .ok_or(WorkerError::JobNotFound)?;
        job.advance(JobEvent::Failed { reason })?;
        if state
            .pending_final
            .as_ref()
            .is_some_and(|p| p.draft.key() == key)
        {
            state.pending_final = None;
        }
        state.pending_failure = Some(PendingFailure {
            key: key.clone(),
            reason,
            unverified,
        });
        self.store.save_state(&state)?;
        self.backfill_failure(key).map(|_| ())
    }
}
