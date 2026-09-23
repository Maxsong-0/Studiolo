//! 恢复决定的结构、时间、会话约束与规范审计回执；不据消费锚点授予权限。

use crate::session::{
    SessionError, SessionId,
    database::{required_row, unavailable},
    recovery::model::{Kind, Operation, Proof},
    schema, secrets,
    totp::{
        model::{Factor, Kind as FactorKind},
        store as factor_store,
    },
    validation,
};
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeSet;
use studiolo_audit_ledger::{AuditPhase, AuditRecord};

pub(super) struct Bounds {
    pub floor: i64,
    pub epoch: i64,
    pub next: i64,
    pub factor_next: i64,
    pub store: String,
}

impl Bounds {
    pub fn load(db: &Connection, next: i64) -> Result<Self, SessionError> {
        let (floor, epoch) = db
            .query_row(
                "SELECT clock_floor,epoch FROM auth_meta WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(required_row)?;
        Ok(Self {
            floor,
            epoch,
            next,
            factor_next: factor_store::next(db)?,
            store: schema::store_id(db)?,
        })
    }
}

pub(super) fn digest(value: &str) -> bool {
    value.len() == 64 && secrets::lower_hex(value)
}

pub(super) fn operation(op: &Operation, bounds: &Bounds) -> Result<(), SessionError> {
    op.encoded()?;
    if op.sequence <= 0
        || op.sequence >= bounds.next
        || op.epoch <= 0
        || op.epoch > bounds.epoch
        || op.verified_at < 0
        || op.verified_at > bounds.floor
        || op.deadline <= op.verified_at
        || op
            .verified_at
            .checked_add(300_000)
            .is_none_or(|limit| op.deadline > limit)
        || op.source_enrollment <= 0
        || op.source_enrollment >= bounds.factor_next
        || !digest(&op.source_digest)
        || SessionId::parse(op.session_id.clone()).is_err()
    {
        return Err(SessionError::CorruptStore);
    }
    match op.kind {
        Kind::Rotate => {
            let unique: BTreeSet<_> = op.hashes.iter().collect();
            if op.generation != op.sequence
                || op.hashes.len() != 10
                || unique.len() != 10
                || op.hashes.iter().any(|hash| !digest(hash))
                || op.slot.is_some()
                || op.target.is_some()
            {
                return Err(SessionError::CorruptStore);
            }
        }
        Kind::Replace => {
            if op.generation <= 0
                || op.generation >= op.sequence
                || !op.hashes.is_empty()
                || op.slot.is_none_or(|slot| slot > 9)
            {
                return Err(SessionError::CorruptStore);
            }
            target(op, bounds)?;
        }
    }
    Ok(())
}

fn target(op: &Operation, bounds: &Bounds) -> Result<(), SessionError> {
    let target = op.target.as_ref().ok_or(SessionError::CorruptStore)?;
    let current_step = op.verified_at / 30_000;
    if target.kind
        != (FactorKind::Recovered {
            operation: op.sequence,
        })
        || target.sequence <= op.source_enrollment
        || target.sequence >= bounds.factor_next
        || op.epoch.checked_add(1) != Some(target.epoch)
        || target.session_id.is_some()
        || target.verified_at != op.verified_at
        || target.deadline != op.deadline
        || target.step < 0
        || target.step < current_step.saturating_sub(1)
        || target.step > current_step + 1
        || target.step > bounds.floor / 30_000 + 1
        || target.seed_ref.is_empty()
        || target.seed_ref.len() > 64
        || !target
            .seed_ref
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        || !digest(&target.seed_digest)
        || target.seed_digest == op.source_digest
    {
        return Err(SessionError::CorruptStore);
    }
    target.encoded()?;
    Ok(())
}

pub(super) fn source(op: &Operation, factor: &Factor) -> Result<(), SessionError> {
    let enrollment = factor
        .enrollment
        .as_ref()
        .ok_or(SessionError::CorruptStore)?;
    if op.source_enrollment != enrollment.operation.sequence
        || op.source_digest != factor.seed_digest
        || op.verified_at < enrollment.activated_at
    {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}

pub(super) fn session(db: &Connection, op: &Operation) -> Result<(), SessionError> {
    let row: Option<(i64, i64, i64)> = db
        .query_row(
            "SELECT epoch,absolute_expiry,access_expiry FROM sessions WHERE id=?1",
            [&op.session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(unavailable)?;
    let Some((epoch, absolute, access)) = row else {
        return Err(SessionError::CorruptStore);
    };
    if op.epoch != epoch || op.deadline > absolute || op.deadline > access {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}

pub(super) fn receipts(
    op: &Operation,
    intent: Option<&str>,
    outcome: Option<&str>,
    bounds: &Bounds,
) -> Result<(), SessionError> {
    let intent = intent
        .map(|value| receipt(value, op, AuditPhase::Intent, bounds))
        .transpose()?;
    let outcome = outcome
        .map(|value| receipt(value, op, AuditPhase::Outcome, bounds))
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
    value: &str,
    op: &Operation,
    phase: AuditPhase,
    bounds: &Bounds,
) -> Result<u64, SessionError> {
    if value.is_empty() || value.len() > 8192 {
        return Err(SessionError::CorruptStore);
    }
    validation::check_receipt(value, &op.event(&bounds.store, phase)?)?;
    let record: AuditRecord =
        serde_json::from_str(value).map_err(|_| SessionError::CorruptStore)?;
    Ok(record.offset())
}

pub(super) fn proof(proof: &Proof, bounds: &Bounds) -> Result<(), SessionError> {
    operation(&proof.operation, bounds)?;
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
