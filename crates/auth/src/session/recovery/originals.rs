//! 原证明的有界快照与精确去重；调用者必须在同一 Auth 事务内读取 pending。

use super::super::{SessionError, totp::store as factor_store};
use super::{
    model::{Kind, Operation, Pending, Proof},
    store,
};
use rusqlite::Connection;
use std::collections::BTreeMap;
use studiolo_audit_ledger::{AuditEvent, AuditPhase};

type Original = (AuditEvent, String);
type Originals = BTreeMap<(String, AuditPhase), Original>;

/// 只有数据库中已确认的原回执进入集合；当前待交付决定不在旧证明中重复发送。
pub(super) fn snapshot(
    db: &Connection,
    operation: &Operation,
    store_id: &str,
) -> Result<Vec<Original>, SessionError> {
    let mut originals = base(db, Some(operation), store_id)?;
    let grants = match operation.kind {
        Kind::Rotate => factor_store::grant(db, &operation.session_id)?
            .into_iter()
            .collect(),
        Kind::Replace => factor_store::grants(db)?,
    };
    if grants.len() > 1024 {
        return Err(SessionError::CorruptStore);
    }
    for proof in grants {
        factor_proof(&mut originals, proof, store_id)?;
    }
    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
        let event = operation.event(store_id, phase)?;
        if originals.contains_key(&(event.event_id().as_str().to_owned(), phase)) {
            return Err(SessionError::CorruptStore);
        }
    }
    Ok(originals.into_values().collect())
}

fn recovery_proof(
    originals: &mut Originals,
    proof: Proof,
    store_id: &str,
) -> Result<(), SessionError> {
    for (phase, receipt) in [
        (AuditPhase::Intent, proof.intent_receipt),
        (AuditPhase::Outcome, proof.outcome_receipt),
    ] {
        insert(originals, proof.operation.event(store_id, phase)?, receipt)?;
    }
    Ok(())
}

fn consumed_proof(
    originals: &mut Originals,
    proof: Pending,
    store_id: &str,
) -> Result<(), SessionError> {
    let intent = proof.intent_receipt.ok_or(SessionError::CorruptStore)?;
    let outcome = proof.outcome_receipt.ok_or(SessionError::CorruptStore)?;
    for (phase, receipt) in [(AuditPhase::Intent, intent), (AuditPhase::Outcome, outcome)] {
        insert(originals, proof.operation.event(store_id, phase)?, receipt)?;
    }
    Ok(())
}

fn factor_proof(
    originals: &mut Originals,
    proof: super::super::totp::model::Proof,
    store_id: &str,
) -> Result<(), SessionError> {
    for (phase, receipt) in [
        (AuditPhase::Intent, proof.intent_receipt),
        (AuditPhase::Outcome, proof.outcome_receipt),
    ] {
        insert(originals, proof.operation.event(store_id, phase)?, receipt)?;
    }
    Ok(())
}

fn insert(
    originals: &mut Originals,
    event: AuditEvent,
    receipt: String,
) -> Result<(), SessionError> {
    let key = (event.event_id().as_str().to_owned(), event.phase());
    if let Some((existing_event, existing_receipt)) = originals.get(&key) {
        if existing_event != &event || existing_receipt != &receipt {
            return Err(SessionError::CorruptStore);
        }
        return Ok(());
    }
    // 最多 1024 个会话、10 个已消费码和 3 份固定证明，各两条原事件。
    if originals.len() >= 2 * (1024 + 10 + 3) {
        return Err(SessionError::CorruptStore);
    }
    originals.insert(key, (event, receipt));
    Ok(())
}

// 离线恢复没有候选 Operation，不伪造 Replace 决定来取得所有原证明。
pub(super) fn restore_snapshot(
    db: &Connection,
    store_id: &str,
) -> Result<Vec<Original>, SessionError> {
    let mut originals = base(db, None, store_id)?;
    for proof in factor_store::grants(db)? {
        factor_proof(&mut originals, proof, store_id)?;
    }
    Ok(originals.into_values().collect())
}

fn base(
    db: &Connection,
    skip: Option<&Operation>,
    store_id: &str,
) -> Result<Originals, SessionError> {
    let control = store::control(db)?;
    let mut originals = Originals::new();
    for proof in [control.current, control.changed].into_iter().flatten() {
        recovery_proof(&mut originals, proof, store_id)?;
    }
    for code in store::codes(db)? {
        if let Some(consumed) = code.consumption {
            if skip.is_some_and(|operation| consumed.operation == *operation) {
                continue;
            }
            consumed_proof(&mut originals, consumed, store_id)?;
        }
    }
    if let Some(proof) = factor_store::factor(db)?.and_then(|factor| factor.enrollment) {
        factor_proof(&mut originals, proof, store_id)?;
    }
    Ok(originals)
}
