//! 固定恢复表的有界投影；所有调用均处于 Auth 唯一事务入口内。

use super::super::{
    SessionError,
    database::{required_row, unavailable},
};
use super::model::{self, Code, Control, MAX_OPERATION, MAX_PROOF, Operation, Pending, Proof};
use rusqlite::{Connection, OptionalExtension};

type ControlRow = (
    i64,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
    u16,
);

pub(super) fn control(db: &Connection) -> Result<Control, SessionError> {
    let raw: ControlRow = db.query_row(
        "SELECT next_operation,CASE WHEN octet_length(current_json)<=65536 THEN current_json END,
         CASE WHEN octet_length(change_json)<=65536 THEN change_json END,high_step,
         CASE WHEN octet_length(consumption_json)<=4096 THEN consumption_json END,used_mask
         FROM recovery_control WHERE id=1", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    ).map_err(required_row)?;
    Ok(Control {
        next: raw.0,
        current: raw.1.map(|v| model::decode(v, MAX_PROOF)).transpose()?,
        changed: raw.2.map(|v| model::decode(v, MAX_PROOF)).transpose()?,
        high_step: raw.3,
        consumption: raw.4.map(|v| model::decode(v, MAX_OPERATION)).transpose()?,
        used_mask: raw.5,
    })
}

pub(super) fn next(db: &Connection) -> Result<i64, SessionError> {
    db.query_row(
        "SELECT next_operation FROM recovery_control WHERE id=1",
        [],
        |r| r.get(0),
    )
    .map_err(required_row)
}

pub(super) fn reserve(db: &Connection) -> Result<i64, SessionError> {
    let next = next(db)?;
    let after = next.checked_add(1).ok_or(SessionError::Capacity)?;
    db.execute(
        "UPDATE recovery_control SET next_operation=?1 WHERE id=1",
        [after],
    )
    .map_err(unavailable)?;
    Ok(next)
}

type PendingRow = (String, Option<String>, Option<String>);

pub(super) fn pending(db: &Connection) -> Result<Option<Pending>, SessionError> {
    let raw: Option<PendingRow> = db.query_row(
        "SELECT CASE WHEN octet_length(operation_json)<=4096 THEN operation_json END,
         CASE WHEN octet_length(intent_receipt)<=8192 THEN intent_receipt END,
         CASE WHEN octet_length(outcome_receipt)<=8192 THEN outcome_receipt END FROM recovery_pending WHERE id=1",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional().map_err(unavailable)?;
    raw.map(|(op, intent, outcome)| {
        Ok(Pending {
            operation: model::decode(op, MAX_OPERATION)?,
            intent_receipt: intent,
            outcome_receipt: outcome,
        })
    })
    .transpose()
}

pub(super) fn codes(db: &Connection) -> Result<Vec<Code>, SessionError> {
    let mut statement = db
        .prepare(
            "SELECT generation,slot,CASE WHEN octet_length(digest)=64 THEN digest END,
         CASE WHEN octet_length(consumption_json)<=4096 THEN consumption_json END,
         CASE WHEN octet_length(intent_receipt)<=8192 THEN intent_receipt END,
         CASE WHEN octet_length(outcome_receipt)<=8192 THEN outcome_receipt END
         FROM recovery_codes ORDER BY generation,slot LIMIT 21",
        )
        .map_err(unavailable)?;
    let mut rows = statement.query([]).map_err(unavailable)?;
    let mut codes = Vec::new();
    while let Some(row) = rows.next().map_err(unavailable)? {
        let consumed: Option<String> = row.get(3).map_err(unavailable)?;
        let intent = row.get(4).map_err(unavailable)?;
        let outcome = row.get(5).map_err(unavailable)?;
        let consumption = consumed
            .map(|op| {
                Ok::<_, SessionError>(Pending {
                    operation: model::decode(op, MAX_OPERATION)?,
                    intent_receipt: intent,
                    outcome_receipt: outcome,
                })
            })
            .transpose()?;
        codes.push(Code {
            generation: row.get(0).map_err(unavailable)?,
            slot: row.get(1).map_err(unavailable)?,
            digest: row.get(2).map_err(unavailable)?,
            consumption,
        });
    }
    Ok(codes)
}

pub(super) fn proof(pending: &Pending, now: i64) -> Result<Proof, SessionError> {
    Ok(Proof {
        operation: pending.operation.clone(),
        intent_receipt: pending
            .intent_receipt
            .clone()
            .ok_or(SessionError::AuditUnavailable)?,
        outcome_receipt: pending
            .outcome_receipt
            .clone()
            .ok_or(SessionError::AuditUnavailable)?,
        activated_at: now,
    })
}

pub(super) fn insert_pending(db: &Connection, operation: &Operation) -> Result<(), SessionError> {
    db.execute(
        "INSERT INTO recovery_pending(id,operation_json) VALUES(1,?1)",
        [operation.encoded()?],
    )
    .map_err(unavailable)?;
    Ok(())
}
