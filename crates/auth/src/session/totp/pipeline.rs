//! 两个固定原事件的幂等交付；迟到回执不会重建状态或激活权限。

use super::super::{
    SessionError, SessionState, database::unavailable, outbox, rows, schema, state,
};
use super::{
    Context,
    model::{Kind, Operation},
    store,
};
use rusqlite::{Connection, params};
use studiolo_audit_ledger::{AuditPhase, AuditRecord};

impl Context {
    pub(super) fn deliver(&self, expected: Option<&Operation>) -> Result<usize, SessionError> {
        let snapshot = self
            .sessions
            .database
            .transaction(|db| Ok((store::pending(db)?, schema::store_id(db)?)))?;
        let (Some(snapshot), store_id) = snapshot else {
            return if expected.is_some() {
                Err(SessionError::Rejected)
            } else {
                Ok(0)
            };
        };
        if expected.is_some_and(|op| op != &snapshot.operation) {
            return Err(SessionError::Rejected);
        }
        // 原因子与待替换的旧会话证明也必须仍由同一Ledger承认，不能混入另一空审计库。
        let originals = self.sessions.database.transaction(|db| {
            let mut proofs = Vec::new();
            if let Some(proof) = store::factor(db)?.and_then(|f| f.enrollment) {
                proofs.push(proof);
            }
            if let Some(id) = &snapshot.operation.session_id
                && let Some(proof) = store::grant(db, id)?
            {
                proofs.push(proof);
            }
            Ok(proofs)
        })?;
        for proof in originals {
            for (phase, prior) in [
                (AuditPhase::Intent, proof.intent_receipt),
                (AuditPhase::Outcome, proof.outcome_receipt),
            ] {
                let event = proof.operation.event(&store_id, phase)?;
                let record = self
                    .sessions
                    .audit
                    .append(event.clone())
                    .map_err(|_| SessionError::AuditUnavailable)?;
                if outbox::receipt(&record, &event)? != prior {
                    return Err(SessionError::AuditUnavailable);
                }
            }
        }
        let mut delivered = 0;
        for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
            let prior = match phase {
                AuditPhase::Intent => &snapshot.intent_receipt,
                AuditPhase::Outcome => &snapshot.outcome_receipt,
            };
            let event = snapshot.operation.event(&store_id, phase)?;
            let record = self
                .sessions
                .audit
                .append(event.clone())
                .map_err(|_| SessionError::AuditUnavailable)?;
            let receipt = outbox::receipt(&record, &event)?;
            if prior.as_ref().is_some_and(|old| old != &receipt) {
                return Err(SessionError::AuditUnavailable);
            }
            let acknowledged = self
                .sessions
                .database
                .transaction(|db| acknowledge(db, &snapshot.operation, phase, &receipt))?;
            if !acknowledged {
                return if expected.is_some() {
                    Err(SessionError::Rejected)
                } else {
                    Ok(delivered)
                };
            }
            delivered += usize::from(prior.is_none());
        }
        Ok(delivered)
    }
}

fn acknowledge(
    db: &Connection,
    operation: &Operation,
    phase: AuditPhase,
    receipt: &str,
) -> Result<bool, SessionError> {
    let Some(current) = store::pending(db)? else {
        return Ok(false);
    };
    if &current.operation != operation {
        return Ok(false);
    }
    let (existing, query) = match phase {
        AuditPhase::Intent => (
            &current.intent_receipt,
            "UPDATE totp_pending SET intent_receipt=?1 WHERE id=1 AND operation_json=?2",
        ),
        AuditPhase::Outcome => {
            let intent = current
                .intent_receipt
                .as_ref()
                .ok_or(SessionError::CorruptStore)?;
            let intent: AuditRecord =
                serde_json::from_str(intent).map_err(|_| SessionError::CorruptStore)?;
            let outcome: AuditRecord =
                serde_json::from_str(receipt).map_err(|_| SessionError::AuditUnavailable)?;
            if outcome.offset() <= intent.offset() {
                return Err(SessionError::AuditUnavailable);
            }
            (
                &current.outcome_receipt,
                "UPDATE totp_pending SET outcome_receipt=?1 WHERE id=1 AND operation_json=?2",
            )
        }
    };
    if let Some(existing) = existing {
        if existing != receipt {
            return Err(SessionError::AuditUnavailable);
        }
        return Ok(true);
    }
    if db
        .execute(query, params![receipt, operation.encoded()?])
        .map_err(unavailable)?
        != 1
    {
        return Err(SessionError::CorruptStore);
    }
    super::schema::validate(db)?;
    Ok(true)
}

pub(super) fn release_invalid(
    db: &Connection,
    observed: &state::Observed,
) -> Result<(), SessionError> {
    let Some(pending) = store::pending(db)? else {
        return Ok(());
    };
    if pending.intent_receipt.is_none() || pending.outcome_receipt.is_none() {
        return Ok(());
    }
    let mut invalid =
        observed.now >= pending.operation.deadline || observed.epoch != pending.operation.epoch;
    if pending.operation.kind == Kind::StepUp {
        let id = super::super::SessionId::parse(
            pending
                .operation
                .session_id
                .clone()
                .ok_or(SessionError::CorruptStore)?,
        )?;
        invalid |= rows::by_id(db, &id)?
            .is_none_or(|r| r.info.state != SessionState::Active || r.epoch != observed.epoch);
    }
    if invalid {
        // 消费锚点与last_step仍在；清理仅释放失效且双确认的决定，绝不重新授权。
        db.execute("DELETE FROM totp_pending WHERE id=1", [])
            .map_err(unavailable)?;
    }
    Ok(())
}
