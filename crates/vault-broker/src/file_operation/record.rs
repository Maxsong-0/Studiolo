//! 唯一主日志中的文件操作分支；类型化终态只记录已知效果，不重建执行历史。
use super::{
    audit_model::FileOperationKind, frozen::Frozen, model::*, observation::Observation,
    status_model::FileOperationPhase,
};
use crate::{BrokerError, VaultUuid, blob::file::ContentProof, mutation::record::Stage};
use serde::{Deserialize, Serialize};
mod terminal;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Active {
    pub frozen: Frozen,
    pub sequence: u64,
    pub reserved_bytes: u64,
    pub phase: FileOperationPhase,
    pub before: Observation,
    pub rejected: bool,
    pub history: Option<ContentProof>,
    pub stage: Option<Stage>,
    pub sealed_stage: Option<ContentProof>,
    pub executed: Option<Observation>,
    pub quarantine: Option<ContentProof>,
    pub effect_at_ms: Option<i64>,
    pub terminal: Option<Terminal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rename: Option<super::rename_model::Progress>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Terminal {
    pub format_version: u32,
    pub frozen: Frozen,
    pub observed_before: FilePresence,
    pub receipt: FileOperationReceipt,
    pub effect_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rename: Option<super::rename_model::TerminalProgress>,
}

impl Active {
    pub(crate) fn validate(&self, pair: VaultUuid) -> Result<(), BrokerError> {
        self.frozen.validate(pair)?;
        self.before.validate()?;
        if self.sequence == 0
            || self.reserved_bytes == 0
            || self.reserved_bytes > self.frozen.limits.retained_bytes_limit
            || self.effect_at_ms.is_some_and(|value| value < 0)
        {
            return Err(BrokerError::CorruptWriterState);
        }
        if matches!(self.frozen.kind, FileOperationKind::Rename { .. }) {
            return super::rename_validate::active(self, pair);
        }
        if self.rename.is_some()
            || matches!(
                self.phase,
                FileOperationPhase::TargetInstallIssued
                    | FileOperationPhase::TargetInstalled
                    | FileOperationPhase::SourceDetachIssued
                    | FileOperationPhase::SourceDetached
            )
        {
            return Err(BrokerError::CorruptWriterState);
        }
        let before = self.before.presence()?;
        let matched = match (&self.frozen.kind, &before) {
            (FileOperationKind::Delete { expected }, FilePresence::Present { revision, .. }) => {
                expected == revision
            }
            (FileOperationKind::RestoreTombstone { .. }, FilePresence::Missing) => true,
            _ => false,
        };
        if self.rejected == matched {
            return Err(BrokerError::CorruptWriterState);
        }
        if self.rejected
            && (self.history.is_some()
                || self.stage.is_some()
                || self.sealed_stage.is_some()
                || self.executed.is_some()
                || self.quarantine.is_some())
        {
            return Err(BrokerError::CorruptWriterState);
        }
        use FileOperationPhase::*;
        if !matches!(self.phase, Preparing | Prepared | FinalizedAwaitAudit) && self.rejected {
            return Err(BrokerError::CorruptWriterState);
        }
        let cancelled_before_copy = self
            .terminal
            .as_ref()
            .is_some_and(|value| value.receipt.outcome == FileOperationOutcome::Cancelled);
        if !self.rejected
            && self.phase != Preparing
            && self.history.is_none()
            && !cancelled_before_copy
        {
            return Err(BrokerError::CorruptWriterState);
        }
        match (&self.frozen.kind, self.phase) {
            (FileOperationKind::Delete { .. }, InstallIssued | Installed)
            | (FileOperationKind::RestoreTombstone { .. }, DetachIssued | Detached) => {
                return Err(BrokerError::CorruptWriterState);
            }
            _ => {}
        }
        if matches!(self.phase, Detached | Installed) && self.executed.is_none() {
            return Err(BrokerError::CorruptWriterState);
        }
        if (self.phase == FinalizedAwaitAudit) != self.terminal.is_some() {
            return Err(BrokerError::CorruptWriterState);
        }
        if let Some(proof) = &self.history {
            Observation::Present {
                proof: proof.clone(),
            }
            .validate()?;
            match self.frozen.kind {
                FileOperationKind::Delete { expected }
                    if proof.revision != Some(expected)
                        || self
                            .before
                            .proof()
                            .is_none_or(|old| old.identity != proof.identity) =>
                {
                    return Err(BrokerError::CorruptWriterState);
                }
                _ => {}
            }
        }
        if self.stage.is_some() != self.sealed_stage.is_some() {
            return Err(BrokerError::CorruptWriterState);
        }
        if let (Some(stage), Some(proof)) = (&self.stage, &self.sealed_stage) {
            Observation::Present {
                proof: proof.clone(),
            }
            .validate()?;
            if !crate::writer::disk::hex(stage.name.strip_suffix(".stage").unwrap_or(""))
                || proof.node.file.identity != stage.identity
                || proof.identity.sha256 != stage.digest
                || proof.identity.bytes != stage.size
                || self
                    .history
                    .as_ref()
                    .is_none_or(|h| h.identity != proof.identity || h.revision != proof.revision)
                || !matches!(self.frozen.kind, FileOperationKind::RestoreTombstone { .. })
            {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        if let Some(executed) = &self.executed {
            executed.validate()?;
        }
        if let Some(proof) = &self.quarantine {
            Observation::Present {
                proof: proof.clone(),
            }
            .validate()?;
            if !matches!(self.frozen.kind, FileOperationKind::Delete { .. })
                || self.before.proof().is_none_or(|old| {
                    old.identity != proof.identity
                        || old.node.file.identity != proof.node.file.identity
                })
            {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        let committed = self
            .terminal
            .as_ref()
            .is_some_and(|v| v.receipt.outcome == FileOperationOutcome::Committed);
        if matches!(self.phase, Detached | Installed) || committed {
            match self.frozen.kind {
                FileOperationKind::Delete { .. }
                    if self.quarantine.is_none()
                        || !matches!(self.executed, Some(Observation::Missing { .. })) =>
                {
                    return Err(BrokerError::CorruptWriterState);
                }
                FileOperationKind::RestoreTombstone { .. }
                    if self.stage.is_none()
                        || self
                            .executed
                            .as_ref()
                            .and_then(|v| v.proof())
                            .is_none_or(|proof| {
                                self.sealed_stage.as_ref().is_none_or(|stage| {
                                    stage.identity != proof.identity
                                        || stage.node.file.identity != proof.node.file.identity
                                })
                            }) =>
                {
                    return Err(BrokerError::CorruptWriterState);
                }
                _ => {}
            }
        }
        if let Some(terminal) = &self.terminal {
            terminal.validate(pair)?;
            if terminal.frozen != self.frozen
                || terminal.receipt.sequence != self.sequence
                || terminal.observed_before != before
                || (committed
                    && !matches!(terminal.receipt.effects.first(), Some(FileEffect::Confirmed { after, .. })
                    if self.executed.as_ref().is_some_and(|v| v.presence().is_ok_and(|actual| &actual == after))))
            {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        Ok(())
    }
    pub(crate) fn blocked_read(&self) -> bool {
        !matches!(
            self.phase,
            FileOperationPhase::Preparing | FileOperationPhase::Prepared
        )
    }
}

pub(crate) fn validate_presence(value: &FilePresence) -> Result<(), BrokerError> {
    if let FilePresence::Present { sha256, bytes, .. } = value
        && (!crate::writer::disk::hex(sha256) || *bytes > crate::blob::OBJECT_LIMIT)
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(())
}
