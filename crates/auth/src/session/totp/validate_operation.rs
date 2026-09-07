//! 决定的时间、种子绑定、会话关联和双阶段原回执检查。

use crate::session::{
    SessionError, SessionId,
    database::unavailable,
    totp::model::{Factor, Kind, Operation, Proof},
    validation,
};
use rusqlite::{Connection, OptionalExtension};
use studiolo_audit_ledger::{AuditPhase, AuditRecord};

pub(super) struct Bounds {
    pub version: i64,
    pub floor: i64,
    pub epoch: i64,
    pub next: i64,
    pub store_id: String,
}

pub(super) fn operation(
    operation: &Operation,
    factor: &Factor,
    bounds: &Bounds,
) -> Result<(), SessionError> {
    let current_step = operation.verified_at / 30_000;
    if operation.sequence <= 0
        || operation.sequence >= bounds.next
        || operation.epoch < 1
        || operation.epoch > bounds.epoch
        || operation.verified_at < 0
        || operation.verified_at > bounds.floor
        || operation.step < 0
        || operation.step > bounds.floor / 30_000 + 1
        || operation.step < current_step.saturating_sub(1)
        || operation.step > current_step + 1
        || operation.deadline <= operation.verified_at
        || operation
            .verified_at
            .checked_add(300_000)
            .is_none_or(|limit| operation.deadline > limit)
        || operation.seed_ref != factor.seed_ref
        || operation.seed_digest != factor.seed_digest
        || operation.step > factor.last_step
    {
        return Err(SessionError::CorruptStore);
    }
    match (&operation.kind, &operation.session_id) {
        (Kind::Enroll, None) => Ok(()),
        (Kind::StepUp, Some(id)) if SessionId::parse(id.clone()).is_ok() => Ok(()),
        (Kind::Recovered { operation }, None) if bounds.version >= 4 && *operation > 0 => Ok(()),
        _ => Err(SessionError::CorruptStore),
    }
}

pub(super) fn session(db: &Connection, operation: &Operation) -> Result<(), SessionError> {
    let id = operation
        .session_id
        .as_ref()
        .ok_or(SessionError::CorruptStore)?;
    let row: Option<(i64, i64, i64)> = db
        .query_row(
            "SELECT epoch,absolute_expiry,access_expiry FROM sessions WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(unavailable)?;
    let Some((epoch, absolute, access)) = row else {
        return Err(SessionError::CorruptStore);
    };
    if operation.epoch != epoch || operation.deadline > absolute || operation.deadline > access {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}

pub(super) fn receipts(
    operation: &Operation,
    intent: Option<&str>,
    outcome: Option<&str>,
    bounds: &Bounds,
) -> Result<(), SessionError> {
    let intent = intent
        .map(|value| receipt(value, operation, AuditPhase::Intent, bounds))
        .transpose()?;
    let outcome = outcome
        .map(|value| receipt(value, operation, AuditPhase::Outcome, bounds))
        .transpose()?;
    if outcome.is_some() && intent.is_none() {
        return Err(SessionError::CorruptStore);
    }
    if let (Some(intent), Some(outcome)) = (intent, outcome)
        && intent >= outcome
    {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}

fn receipt(
    encoded: &str,
    operation: &Operation,
    phase: AuditPhase,
    bounds: &Bounds,
) -> Result<u64, SessionError> {
    validation::check_receipt(encoded, &operation.event(&bounds.store_id, phase)?)?;
    let record: AuditRecord =
        serde_json::from_str(encoded).map_err(|_| SessionError::CorruptStore)?;
    Ok(record.offset())
}

pub(super) fn proof(proof: &Proof, factor: &Factor, bounds: &Bounds) -> Result<(), SessionError> {
    operation(&proof.operation, factor, bounds)?;
    if proof.activated_at < proof.operation.verified_at
        || proof.activated_at >= proof.operation.deadline
        || proof.activated_at > bounds.floor
    {
        return Err(SessionError::CorruptStore);
    }
    receipts(
        &proof.operation,
        Some(&proof.intent_receipt),
        Some(&proof.outcome_receipt),
        bounds,
    )
}

pub(super) fn after_enrollment(operation: &Operation, factor: &Factor) -> Result<(), SessionError> {
    let enrollment = factor
        .enrollment
        .as_ref()
        .ok_or(SessionError::CorruptStore)?;
    if operation.sequence <= enrollment.operation.sequence
        || operation.step <= enrollment.operation.step
        || operation.verified_at < enrollment.activated_at
    {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}
