use super::{MAX_JOBS, MAX_QUEUED, PersistedState, StoreError};
use crate::worker::{
    final_manifest::FinalManifestReceipt,
    recovery_point::RecoveryPointStamps,
    state::{JobRecord, JobState, UnknownStage},
};

pub(super) fn validate(state: &PersistedState) -> Result<(), StoreError> {
    let bad = StoreError::CorruptStore;
    if state.jobs.len() > MAX_JOBS
        || state
            .jobs
            .iter()
            .filter(|j| j.state == JobState::Queued)
            .count()
            > MAX_QUEUED
    {
        return Err(StoreError::LimitExceeded);
    }
    if state.next_sequence == 0 {
        return Err(bad);
    }
    let mut previous = 0;
    let mut active = None;
    for job in &state.jobs {
        job.validate().map_err(|_| bad)?;
        if job.key.sequence.get() <= previous || job.key.sequence.get() >= state.next_sequence {
            return Err(bad);
        }
        previous = job.key.sequence.get();
        if job.state != JobState::Queued
            && !job.state.is_terminal()
            && active.replace(job).is_some()
        {
            return Err(bad);
        }
        if !matches!(
            job.state,
            JobState::Queued
                | JobState::Failed { .. }
                | JobState::Sealing
                | JobState::ResultUnknown {
                    stage: UnknownStage::Sealing
                }
        ) && job.source.is_none()
        {
            return Err(bad);
        }
    }
    match (&state.pending_final, active) {
        (Some(pending), Some(job)) => {
            if pending.draft.key() != &job.key
                || job.source.as_ref() != Some(pending.draft.source())
                || job.checkpoint_at != Some(pending.draft.manifest().content_cutoff)
                || job.remote_verified_at != Some(pending.draft.data_verified_at())
                || !matches!(
                    job.state,
                    JobState::CommittingFinalManifest
                        | JobState::AuditPending { .. }
                        | JobState::ResultUnknown {
                            stage: UnknownStage::FinalManifestAck
                        }
                )
            {
                return Err(bad);
            }
            if let Some(receipt) = &pending.receipt {
                if receipt.draft() != &pending.draft {
                    return Err(bad);
                }
                if matches!(job.state, JobState::AuditPending { .. })
                    && job.finalized_at != Some(receipt.finalized_at())
                {
                    return Err(bad);
                }
            } else if matches!(job.state, JobState::AuditPending { .. }) {
                return Err(bad);
            }
        }
        (None, Some(job))
            if matches!(
                job.state,
                JobState::CommittingFinalManifest
                    | JobState::AuditPending { .. }
                    | JobState::ResultUnknown {
                        stage: UnknownStage::FinalManifestAck
                    }
            ) =>
        {
            return Err(bad);
        }
        (Some(_), None) => return Err(bad),
        _ => {}
    }
    if let Some(pending) = &state.pending_failure {
        let job = state
            .jobs
            .iter()
            .find(|j| j.key == pending.key)
            .ok_or(bad)?;
        if active.is_some()
            || state.pending_final.is_some()
            || job.state
                != (JobState::Failed {
                    reason: pending.reason,
                })
            || (pending.unverified.is_some()
                != (pending.reason == crate::worker::state::FailureReason::TargetUnverified))
        {
            return Err(bad);
        }
    }
    match (&state.last_final, state.last_scheduled) {
        (Some(receipt), Some(stamps)) => {
            let job = state
                .jobs
                .iter()
                .find(|j| &j.key == receipt.draft().key())
                .ok_or(bad)?;
            if job.state != JobState::Succeeded
                || !matches_receipt(job, receipt)
                || stamps != receipt_stamps(receipt)
            {
                return Err(bad);
            }
            if state
                .jobs
                .iter()
                .any(|j| j.state == JobState::Succeeded && j.key.sequence > job.key.sequence)
            {
                return Err(bad);
            }
        }
        (None, None) if !state.jobs.iter().any(|j| j.state == JobState::Succeeded) => {}
        _ => return Err(bad),
    }
    if state.seed_verified && state.last_final.is_none() {
        return Err(bad);
    }
    if state.last_daily_immutable.is_some_and(|s| {
        s.checkpoint_at.millis() < 0
            || s.remote_verified_at < s.checkpoint_at
            || s.finalized_at < s.remote_verified_at
    }) {
        return Err(bad);
    }
    Ok(())
}

fn matches_receipt(job: &JobRecord, receipt: &FinalManifestReceipt) -> bool {
    let stamps = receipt_stamps(receipt);
    job.source.as_ref() == Some(receipt.draft().source())
        && job.checkpoint_at == Some(stamps.checkpoint_at)
        && job.remote_verified_at == Some(stamps.remote_verified_at)
        && job.finalized_at == Some(stamps.finalized_at)
}
pub(crate) fn receipt_stamps(receipt: &FinalManifestReceipt) -> RecoveryPointStamps {
    RecoveryPointStamps {
        checkpoint_at: receipt.draft().manifest().content_cutoff,
        remote_verified_at: receipt.draft().data_verified_at(),
        finalized_at: receipt.finalized_at(),
    }
}
