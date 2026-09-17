//! 终态验证只接受类型/效果一致的固定事实。
use super::*;

impl Terminal {
    pub(crate) fn validate(&self, pair: VaultUuid) -> Result<(), BrokerError> {
        self.frozen.validate(pair)?;
        if matches!(self.frozen.kind, FileOperationKind::Rename { .. }) {
            return super::super::rename_terminal::validate(self);
        }
        if self.rename.is_some()
            || self.format_version != 5
            || self.receipt.operation_id != self.frozen.operation_id
            || self.receipt.sequence == 0
            || self.receipt.effects.len() != 1
            || self.effect_at_ms.is_some_and(|value| value < 0)
        {
            return Err(BrokerError::CorruptWriterState);
        }
        if self.receipt.outcome == FileOperationOutcome::Rejected {
            let matched = match (&self.frozen.kind, &self.observed_before) {
                (
                    FileOperationKind::Delete { expected },
                    FilePresence::Present { revision, .. },
                ) => expected == revision,
                (FileOperationKind::RestoreTombstone { .. }, FilePresence::Missing) => true,
                _ => false,
            };
            if matched {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        let effect = &self.receipt.effects[0];
        match (self.receipt.outcome, effect) {
            (
                FileOperationOutcome::Committed,
                FileEffect::Confirmed {
                    path,
                    before,
                    after,
                },
            ) => {
                if path != &self.frozen.path
                    || before != &self.observed_before
                    || self.receipt.evidence_id.is_some()
                {
                    return Err(BrokerError::CorruptWriterState);
                }
                match (&self.frozen.kind, before, after) {
                    (
                        FileOperationKind::Delete { expected },
                        FilePresence::Present { revision, .. },
                        FilePresence::Missing,
                    ) if expected == revision
                        && self.receipt.tombstone_id.as_ref()
                            == Some(&self.frozen.operation_id) => {}
                    (
                        FileOperationKind::RestoreTombstone { .. },
                        FilePresence::Missing,
                        FilePresence::Present { .. },
                    ) if self.receipt.tombstone_id.is_none() => {}
                    _ => return Err(BrokerError::CorruptWriterState),
                }
            }
            (
                FileOperationOutcome::Rejected | FileOperationOutcome::Cancelled,
                FileEffect::NotApplied { path },
            ) if path == &self.frozen.path
                && self.receipt.tombstone_id.is_none()
                && self.receipt.evidence_id.is_none() => {}
            (FileOperationOutcome::Indeterminate, FileEffect::Unknown { path })
                if path == &self.frozen.path
                    && self.receipt.tombstone_id.is_none()
                    && self
                        .receipt
                        .evidence_id
                        .as_ref()
                        .is_some_and(|id| crate::writer::disk::hex(id)) => {}
            _ => return Err(BrokerError::CorruptWriterState),
        }
        validate_presence(&self.observed_before)?;
        if let FileEffect::Confirmed { before, after, .. } = effect {
            validate_presence(before)?;
            validate_presence(after)?;
        }
        Ok(())
    }
}
