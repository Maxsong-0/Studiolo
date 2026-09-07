//! 精确重放原事件；双回执只确认决定，绝不从补记路径发布恢复码或切换因子。

use super::super::{
    SessionError, SessionId, SessionState, database::unavailable, outbox, rows,
    schema as session_schema, state,
};
use super::{
    Recovery,
    model::{Kind, Operation},
    mutations, originals, store,
};
use rusqlite::{Connection, params};
use studiolo_audit_ledger::{AuditPhase, AuditRecord};

impl Recovery {
    pub(super) fn deliver(&self, expected: Option<&Operation>) -> Result<usize, SessionError> {
        let snapshot = self.sessions.database.transaction(|db| {
            let Some(pending) = store::pending(db)? else {
                return Ok(None);
            };
            if expected.is_some_and(|operation| operation != &pending.operation) {
                return Err(SessionError::Rejected);
            }
            let store_id = session_schema::store_id(db)?;
            let originals = originals::snapshot(db, &pending.operation, &store_id)?;
            Ok(Some((pending, store_id, originals)))
        })?;
        let Some((snapshot, store_id, originals)) = snapshot else {
            return if expected.is_some() {
                Err(SessionError::Rejected)
            } else {
                Ok(0)
            };
        };
        // 所有外部调用均在 Auth 锁外；去重后的旧回执也必须逐字保持一致。
        for (event, prior) in originals {
            let record = self
                .sessions
                .audit
                .append(event.clone())
                .map_err(|_| SessionError::AuditUnavailable)?;
            if outbox::receipt(&record, &event)? != prior {
                return Err(SessionError::AuditUnavailable);
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
            if prior.as_ref().is_some_and(|existing| existing != &receipt) {
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
            // 与 TOTP 一致：只计本次快照中原先尚未确认的当前事件，不计旧证据重放。
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
    let (existing, query, mirror) = match phase {
        AuditPhase::Intent => (
            &current.intent_receipt,
            "UPDATE recovery_pending SET intent_receipt=?1 WHERE id=1 AND operation_json=?2 AND intent_receipt IS NULL",
            "UPDATE recovery_codes SET intent_receipt=?1 WHERE generation=?3 AND slot=?4 AND consumption_json=?2 AND intent_receipt IS NULL",
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
                "UPDATE recovery_pending SET outcome_receipt=?1 WHERE id=1 AND operation_json=?2 AND outcome_receipt IS NULL",
                "UPDATE recovery_codes SET outcome_receipt=?1 WHERE generation=?3 AND slot=?4 AND consumption_json=?2 AND outcome_receipt IS NULL",
            )
        }
    };
    if let Some(existing) = existing {
        if existing != receipt {
            return Err(SessionError::AuditUnavailable);
        }
        super::schema::validate(db)?;
        return Ok(true);
    }
    let encoded = operation.encoded()?;
    if db
        .execute(query, params![receipt, encoded])
        .map_err(unavailable)?
        != 1
    {
        return Err(SessionError::CorruptStore);
    }
    if operation.kind == Kind::Replace {
        let slot = operation.slot.ok_or(SessionError::CorruptStore)?;
        if db
            .execute(
                mirror,
                params![receipt, encoded, operation.generation, slot],
            )
            .map_err(unavailable)?
            != 1
        {
            return Err(SessionError::CorruptStore);
        }
    }
    // 镜像与 pending 同事务更新后再做全字段关联验证；失败时二者一起回滚。
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
    let id = SessionId::parse(pending.operation.session_id.clone())?;
    let invalid = observed.now >= pending.operation.deadline
        || observed.epoch != pending.operation.epoch
        || rows::by_id(db, &id)?.is_none_or(|row| {
            row.info.state != SessionState::Active || row.epoch != observed.epoch
        });
    if invalid {
        // 只释放双确认的失效决定；Replace 的码消费、位图和高水位永久保留。
        mutations::discard(db, &pending)?;
        super::schema::validate(db)?;
    }
    Ok(())
}
