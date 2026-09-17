//! Rename 私有状态闭集；旧单路径校验不会为新增阶段默认提供成功语义。
use super::{observation::Observation, record::Active, *};
use crate::{BrokerError, VaultUuid, blob::file::ContentProof};
fn bad() -> BrokerError {
    BrokerError::CorruptWriterState
}
fn proof(p: &ContentProof) -> Result<(), BrokerError> {
    Observation::Present { proof: p.clone() }.validate()
}
pub(crate) fn active(a: &Active, pair: VaultUuid) -> Result<(), BrokerError> {
    use FileOperationPhase::*;
    let (_, r) = rename_model::get(a)?;
    r.before.validate()?;
    let FileOperationKind::Rename { expected, .. } = a.frozen.kind else {
        return Err(bad());
    };
    let matched = a
        .before
        .proof()
        .is_some_and(|p| p.revision == Some(expected))
        && matches!(r.before, Observation::Missing { .. });
    let body = if a.rejected {
        0
    } else {
        a.before.proof().ok_or_else(bad)?.identity.bytes
    };
    if a.rejected == matched
        || a.reserved_bytes != rename_capacity::reservation(body, !a.rejected)?
        || r.install_id != rename_model::step(&a.frozen.operation_id, "target-install")
        || r.detach_id != rename_model::step(&a.frozen.operation_id, "source-detach")
        || r.target_effect_at_ms.is_some_and(|v| v < 0)
    {
        return Err(bad());
    }
    let phase = if a.phase == FinalizedAwaitAudit {
        a.terminal
            .as_ref()
            .and_then(|t| t.rename.as_ref())
            .ok_or_else(bad)?
            .phase
    } else {
        a.phase
    };
    if !matches!(
        phase,
        Preparing
            | Prepared
            | TargetInstallIssued
            | TargetInstalled
            | SourceDetachIssued
            | SourceDetached
    ) || (a.phase == FinalizedAwaitAudit) != a.terminal.is_some()
    {
        return Err(bad());
    }
    let before_issue = matches!(phase, Preparing | Prepared);
    if a.rejected
        && (!before_issue
            || a.history.is_some()
            || r.history.is_some()
            || a.stage.is_some()
            || r.target_revision.is_some())
    {
        return Err(bad());
    }
    if a.stage.is_some() != r.target_revision.is_some()
        || a.stage.is_some() != a.sealed_stage.is_some()
    {
        return Err(bad());
    }
    let cancelled = a
        .terminal
        .as_ref()
        .is_some_and(|t| t.receipt.outcome == FileOperationOutcome::Cancelled);
    if !a.rejected
        && phase != Preparing
        && !cancelled
        && (a.history.is_none() || r.history.is_none() || a.stage.is_none())
    {
        return Err(bad());
    }
    if let Some(p) = &a.history {
        proof(p)?;
        if p.revision != Some(expected) || a.before.proof().is_none_or(|b| b.identity != p.identity)
        {
            return Err(bad());
        }
    }
    if let Some(p) = &r.history {
        proof(p)?;
        if p.revision != r.target_revision
            || p.revision == Some(expected)
            || a.history.as_ref().is_none_or(|b| b.identity != p.identity)
        {
            return Err(bad());
        }
    }
    if let (Some(stage), Some(p)) = (&a.stage, &a.sealed_stage) {
        proof(p)?;
        let h = a.history.as_ref().ok_or_else(bad)?;
        if !crate::writer::disk::hex(stage.name.strip_suffix(".stage").unwrap_or(""))
            || p.node.file.identity != stage.identity
            || p.identity.sha256 != stage.digest
            || p.identity.bytes != stage.size
            || p.identity != h.identity
            || p.revision != r.target_revision
            || p.revision == Some(expected)
        {
            return Err(bad());
        }
    }
    let target_known = matches!(phase, TargetInstalled | SourceDetachIssued | SourceDetached);
    if target_known != r.executed.is_some() || (!target_known && r.target_effect_at_ms.is_some()) {
        return Err(bad());
    }
    if let Some(o) = &r.executed {
        o.validate()?;
        let p = o.proof().ok_or_else(bad)?;
        let s = a.sealed_stage.as_ref().ok_or_else(bad)?;
        if p.identity != s.identity
            || p.revision != s.revision
            || p.node.file.identity != s.node.file.identity
        {
            return Err(bad());
        }
    }
    let source_issued = matches!(phase, SourceDetachIssued | SourceDetached);
    if source_issued != r.source_check.is_some() {
        return Err(bad());
    }
    if let Some(o) = &r.source_check {
        o.validate()?;
        let p = o.proof().ok_or_else(bad)?;
        let b = a.before.proof().ok_or_else(bad)?;
        if p.identity != b.identity || p.revision != b.revision || p.node.file != b.node.file {
            return Err(bad());
        }
        rename_target::same_parents(&p.node.parents, &b.node.parents).map_err(|_| bad())?;
    }
    let detached = phase == SourceDetached;
    if detached != a.executed.is_some()
        || detached != a.quarantine.is_some()
        || (!detached && a.effect_at_ms.is_some())
    {
        return Err(bad());
    }
    if detached {
        let o = a.executed.as_ref().ok_or_else(bad)?;
        o.validate()?;
        let q = a.quarantine.as_ref().ok_or_else(bad)?;
        proof(q)?;
        let old = a.before.proof().ok_or_else(bad)?;
        if !matches!(o, Observation::Missing { .. })
            || q.revision != Some(expected)
            || q.identity != old.identity
            || q.node.file.identity != old.node.file.identity
        {
            return Err(bad());
        }
    }
    let target_before = r.before.presence()?;
    if let Some(t) = &a.terminal {
        t.validate(pair)?;
        if t.frozen != a.frozen
            || t.observed_before != a.before.presence()?
            || t.receipt.sequence != a.sequence
            || t.rename.as_ref().is_none_or(|v| {
                v.before != target_before
                    || v.target_effect_at_ms != r.target_effect_at_ms
                    || v.target_revision != r.target_revision
            })
            || t.receipt.effects != rename_terminal::effects(a, t.receipt.outcome)?
        {
            return Err(bad());
        }
    }
    Ok(())
}
