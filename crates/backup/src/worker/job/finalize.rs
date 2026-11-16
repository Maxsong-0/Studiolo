use super::{BackupWorker, JobOutcome, WorkerError, payload};
use crate::{
    store::model::StoreError,
    worker::{
        final_manifest::{FinalManifestLookup, FinalManifestReader, FinalManifestReceipt},
        recovery_point::RecoveryPointStamps,
        state::{
            FailureReason, JobEvent, JobKey, JobState, ReconciledOutcome, RemoteVerification,
            UnknownStage,
        },
    },
};

fn verification(receipt: &FinalManifestReceipt) -> RemoteVerification {
    RemoteVerification {
        objects_complete: true,
        checksum_verified: true,
        paired_checkpoint_reference: true,
        verified_at: receipt.draft().data_verified_at(),
    }
}
impl BackupWorker<'_> {
    pub(super) fn accept_receipt(
        &self,
        key: &JobKey,
        receipt: FinalManifestReceipt,
    ) -> Result<JobOutcome, WorkerError> {
        let mut state = self.state()?;
        let pending = state
            .pending_final
            .as_mut()
            .ok_or(WorkerError::EvidenceMismatch)?;
        if pending.draft.key() != key
            || &pending.draft != receipt.draft()
            || pending.receipt.as_ref().is_some_and(|old| old != &receipt)
        {
            return Err(WorkerError::EvidenceMismatch);
        }
        let job = state
            .jobs
            .iter_mut()
            .find(|j| &j.key == key)
            .ok_or(WorkerError::JobNotFound)?;
        if job.state
            == (JobState::ResultUnknown {
                stage: UnknownStage::FinalManifestAck,
            })
        {
            job.reconcile_unknown(
                key,
                ReconciledOutcome::RemoteVerified {
                    verification: verification(&receipt),
                },
            )?;
        }
        if job.state != JobState::CommittingFinalManifest {
            return Err(WorkerError::EvidenceMismatch);
        }
        pending.receipt = Some(receipt.clone());
        // 必须先固定回执，再追加 Outcome；后者重试使用完全相同的 payload。
        job.advance(JobEvent::FinalManifestCommitted {
            finalized_at: receipt.finalized_at(),
            outcome_audit_succeeded: false,
        })?;
        self.store.save_state(&state)?;
        self.backfill_outcome(key)
    }

    /// 仅核对非正文最终记录；允许原本地封存已经过期，绝不再次上传。
    pub fn reconcile_final(
        &self,
        key: &JobKey,
        reader: &dyn FinalManifestReader,
    ) -> Result<JobOutcome, WorkerError> {
        let state = self.state()?;
        let pending = state
            .pending_final
            .as_ref()
            .filter(|p| p.draft.key() == key)
            .ok_or(WorkerError::EvidenceMismatch)?;
        let job = state
            .jobs
            .iter()
            .find(|j| &j.key == key)
            .ok_or(WorkerError::JobNotFound)?;
        if matches!(job.state, JobState::AuditPending { .. }) {
            return self.backfill_outcome(key);
        }
        if !matches!(
            job.state,
            JobState::CommittingFinalManifest
                | JobState::ResultUnknown {
                    stage: UnknownStage::FinalManifestAck
                }
        ) {
            return Err(WorkerError::EvidenceMismatch);
        }
        if let Some(receipt) = &pending.receipt {
            return self.accept_receipt(key, receipt.clone());
        }
        if job.state == JobState::CommittingFinalManifest {
            self.mark_unknown(key, UnknownStage::FinalManifestAck)?;
        }
        match reader.read_final(&pending.draft) {
            Ok(FinalManifestLookup::Verified(receipt)) if receipt.draft() == &pending.draft => {
                self.accept_receipt(key, *receipt)
            }
            Ok(FinalManifestLookup::Absent { observed_at })
                if observed_at >= pending.draft.source_expires_at() =>
            {
                self.finish_failed(key, FailureReason::FinalManifestCommitFailed, None)?;
                Ok(JobOutcome::Failed(FailureReason::FinalManifestCommitFailed))
            }
            _ => Ok(JobOutcome::ResultUnknown(UnknownStage::FinalManifestAck)),
        }
    }

    /// 成功审计、作业终态、最终回执及恢复点指针构成同一次状态替换。
    pub fn backfill_outcome(&self, key: &JobKey) -> Result<JobOutcome, WorkerError> {
        let mut state = self.state()?;
        let receipt = state
            .pending_final
            .as_ref()
            .filter(|p| p.draft.key() == key)
            .and_then(|p| p.receipt.clone())
            .ok_or(WorkerError::EvidenceMismatch)?;
        let job = state
            .jobs
            .iter_mut()
            .find(|j| &j.key == key)
            .ok_or(WorkerError::JobNotFound)?;
        if !matches!(job.state, JobState::AuditPending { .. }) {
            return Err(WorkerError::EvidenceMismatch);
        }
        // 缓存失败不把已经存在的远端记录改判成上传失败；固定回执仍等待本地补记。
        if self
            .store
            .commit_final_manifest(receipt.draft().manifest())
            .is_err()
            || self
                .record_outcome(key, payload::outcome_success(key, &receipt))
                .is_err()
        {
            return Ok(JobOutcome::AuditPending);
        }
        job.advance(JobEvent::OutcomeBackfilled)?;
        if state
            .last_final
            .as_ref()
            .is_some_and(|last| last.draft().key().sequence >= key.sequence)
        {
            return Err(StoreError::CorruptStore.into());
        }
        state.last_scheduled = Some(RecoveryPointStamps {
            checkpoint_at: receipt.draft().manifest().content_cutoff,
            remote_verified_at: receipt.draft().data_verified_at(),
            finalized_at: receipt.finalized_at(),
        });
        state.last_final = Some(receipt);
        state.pending_final = None;
        self.store.save_state(&state)?;
        Ok(JobOutcome::Succeeded)
    }
}
