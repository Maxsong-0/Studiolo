use super::{JobRecord, JobState, StateError, UnknownStage};

pub(super) fn stage_matches(state: JobState, stage: UnknownStage) -> bool {
    matches!(
        (state, stage),
        (JobState::Sealing, UnknownStage::Sealing)
            | (JobState::Uploading, UnknownStage::Uploading)
            | (JobState::Verifying, UnknownStage::Verifying)
            | (
                JobState::CommittingFinalManifest,
                UnknownStage::FinalManifestAck
            )
    )
}

impl JobRecord {
    /// 时间戳必须形成有序前缀，且与状态相符；反序列化状态也调用相同检查。
    pub fn validate(&self) -> Result<(), StateError> {
        let bad = StateError::InconsistentRecord;
        let (c, v, f) = (
            self.checkpoint_at,
            self.remote_verified_at,
            self.finalized_at,
        );
        if c.is_some_and(|t| t.millis() < 0)
            || v.is_some_and(|t| c.is_none_or(|c| t < c))
            || f.is_some_and(|t| v.is_none_or(|v| t < v))
        {
            return Err(bad);
        }
        let shape = (c.is_some(), v.is_some(), f.is_some());
        let correct = match self.state {
            JobState::Queued => shape == (false, false, false),
            JobState::Sealing
            | JobState::Uploading
            | JobState::Verifying
            | JobState::ResultUnknown {
                stage: UnknownStage::Sealing | UnknownStage::Uploading | UnknownStage::Verifying,
            } => shape == (true, false, false),
            JobState::CommittingFinalManifest
            | JobState::ResultUnknown {
                stage: UnknownStage::FinalManifestAck,
            } => shape == (true, true, false),
            JobState::AuditPending { verification } => {
                shape == (true, true, true)
                    && verification.is_recoverable()
                    && v == Some(verification.verified_at)
            }
            JobState::Succeeded => shape == (true, true, true),
            JobState::Failed { .. } => f.is_none(),
        };
        if !correct {
            return Err(bad);
        }
        if let Some(source) = &self.source
            && (source.checkpoint_id != self.key.checkpoint_id
                || !studiolo_core::is_sha256_hex(&source.seal_sha256))
        {
            return Err(bad);
        }
        Ok(())
    }
}
