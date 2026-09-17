//! 双效果固定顺序为目标、源；相同当前内容永远不能补历史安装证明。
use super::{
    record::{Active, Terminal},
    rename_model::TerminalProgress,
    *,
};
use crate::{BrokerError, mutation};
fn bad() -> BrokerError {
    BrokerError::CorruptWriterState
}
pub(crate) fn original_phase(a: &Active) -> Result<FileOperationPhase, BrokerError> {
    if a.phase == FileOperationPhase::FinalizedAwaitAudit {
        Ok(a.terminal
            .as_ref()
            .and_then(|t| t.rename.as_ref())
            .ok_or_else(bad)?
            .phase)
    } else {
        Ok(a.phase)
    }
}
pub(crate) fn effects(
    a: &Active,
    outcome: FileOperationOutcome,
) -> Result<Vec<FileEffect>, BrokerError> {
    let (to, r) = rename_model::get(a)?;
    let from = &a.frozen.path;
    if matches!(
        outcome,
        FileOperationOutcome::Cancelled | FileOperationOutcome::Rejected
    ) {
        return Ok(vec![
            FileEffect::NotApplied { path: to.clone() },
            FileEffect::NotApplied { path: from.clone() },
        ]);
    }
    let target = match &r.executed {
        Some(o) => FileEffect::Confirmed {
            path: to.clone(),
            before: r.before.presence()?,
            after: o.presence()?,
        },
        None => FileEffect::Unknown { path: to.clone() },
    };
    let source = match &a.executed {
        Some(o) => FileEffect::Confirmed {
            path: from.clone(),
            before: a.before.presence()?,
            after: o.presence()?,
        },
        None if original_phase(a)? == FileOperationPhase::SourceDetachIssued => {
            FileEffect::Unknown { path: from.clone() }
        }
        None => FileEffect::NotApplied { path: from.clone() },
    };
    Ok(vec![target, source])
}
pub(crate) fn set(
    state: &mut mutation::State,
    outcome: FileOperationOutcome,
    evidence: Option<String>,
) -> Result<(), BrokerError> {
    let a = state.file_operation.as_mut().ok_or_else(bad)?;
    let (_, r) = rename_model::get(a)?;
    let progress = TerminalProgress {
        before: r.before.presence()?,
        phase: a.phase,
        target_revision: r.target_revision,
        target_effect_at_ms: r.target_effect_at_ms,
    };
    let receipt = FileOperationReceipt {
        operation_id: a.frozen.operation_id.clone(),
        sequence: a.sequence,
        outcome,
        effects: effects(a, outcome)?,
        tombstone_id: (outcome == FileOperationOutcome::Committed)
            .then(|| a.frozen.operation_id.clone()),
        evidence_id: evidence,
    };
    let terminal = Terminal {
        format_version: 6,
        frozen: a.frozen.clone(),
        observed_before: a.before.presence()?,
        effect_at_ms: a.effect_at_ms,
        receipt,
        rename: Some(progress),
    };
    terminal.validate(state.pair_id)?;
    a.terminal = Some(terminal);
    a.phase = FileOperationPhase::FinalizedAwaitAudit;
    if outcome == FileOperationOutcome::Committed {
        state.committed_through = a.sequence;
    }
    Ok(())
}
pub(crate) fn validate(t: &Terminal) -> Result<(), BrokerError> {
    use FileOperationPhase::*;
    let r = t.rename.as_ref().ok_or_else(bad)?;
    let FileOperationKind::Rename { expected, to } = &t.frozen.kind else {
        return Err(bad());
    };
    let c = &t.receipt;
    if t.format_version != 6
        || c.operation_id != t.frozen.operation_id
        || c.sequence == 0
        || c.effects.len() != 2
        || t.effect_at_ms.is_some_and(|v| v < 0)
        || r.target_effect_at_ms.is_some_and(|v| v < 0)
    {
        return Err(bad());
    }
    for p in [&t.observed_before, &r.before] {
        record::validate_presence(p)?;
    }
    if r.target_revision == Some(*expected)
        || (r.phase != Preparing
            && t.receipt.outcome != FileOperationOutcome::Rejected
            && r.target_revision.is_none())
        || (t.receipt.outcome == FileOperationOutcome::Rejected && r.target_revision.is_some())
    {
        return Err(bad());
    }
    let matched = matches!(&t.observed_before,FilePresence::Present{revision,..} if revision==expected)
        && matches!(r.before, FilePresence::Missing);
    for (e, path) in c.effects.iter().zip([to, &t.frozen.path]) {
        let actual = match e {
            FileEffect::Confirmed {
                path,
                before,
                after,
            } => {
                record::validate_presence(before)?;
                record::validate_presence(after)?;
                path
            }
            FileEffect::NotApplied { path } | FileEffect::Unknown { path } => path,
        };
        if actual != path {
            return Err(bad());
        }
    }
    let target_known = matches!(
        r.phase,
        TargetInstalled | SourceDetachIssued | SourceDetached
    );
    let target_valid = matches!(&c.effects[0],FileEffect::Confirmed{before:FilePresence::Missing,after:FilePresence::Present{revision,sha256,bytes},..}
        if Some(*revision)==r.target_revision && revision!=expected && matches!(&t.observed_before,FilePresence::Present{sha256:s,bytes:b,..} if s==sha256&&b==bytes));
    let no_target = matches!(c.effects[0], FileEffect::NotApplied { .. });
    let no_source = matches!(c.effects[1], FileEffect::NotApplied { .. });
    if !target_known && r.target_effect_at_ms.is_some() {
        return Err(bad());
    }
    match c.outcome {
        FileOperationOutcome::Committed => {
            if !matched
                || r.phase != SourceDetached
                || !target_valid
                || !matches!(&c.effects[1],FileEffect::Confirmed{before,after:FilePresence::Missing,..} if before==&t.observed_before)
                || c.tombstone_id.as_ref() != Some(&t.frozen.operation_id)
                || c.evidence_id.is_some()
            {
                return Err(bad());
            }
        }
        FileOperationOutcome::Rejected | FileOperationOutcome::Cancelled => {
            if !matches!(r.phase, Preparing | Prepared)
                || !no_target
                || !no_source
                || c.tombstone_id.is_some()
                || c.evidence_id.is_some()
                || (c.outcome == FileOperationOutcome::Rejected) == matched
                || t.effect_at_ms.is_some()
            {
                return Err(bad());
            }
        }
        FileOperationOutcome::Indeterminate => {
            if !matched
                || c.tombstone_id.is_some()
                || !c
                    .evidence_id
                    .as_ref()
                    .is_some_and(|id| crate::writer::disk::hex(id))
                || t.effect_at_ms.is_some()
            {
                return Err(bad());
            }
            let valid = match r.phase {
                TargetInstallIssued => {
                    matches!(c.effects[0], FileEffect::Unknown { .. }) && no_source
                }
                TargetInstalled => target_valid && no_source,
                SourceDetachIssued => {
                    target_valid && matches!(c.effects[1], FileEffect::Unknown { .. })
                }
                _ => false,
            };
            if !valid {
                return Err(bad());
            }
        }
    }
    Ok(())
}
