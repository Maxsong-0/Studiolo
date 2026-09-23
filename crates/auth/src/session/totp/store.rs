//! 固定表与有界投影，不允许调用者传递 SQL、目录或连接。

use super::super::{
    SessionError,
    database::{required_row, unavailable},
};
use super::model::{Factor, Operation, Pending, Proof};
use rusqlite::{Connection, OptionalExtension, params};

fn operation(value: String) -> Result<Operation, SessionError> {
    let decoded: Operation =
        serde_json::from_str(&value).map_err(|_| SessionError::CorruptStore)?;
    if decoded.encoded()? != value {
        return Err(SessionError::CorruptStore);
    }
    Ok(decoded)
}

pub(in crate::session) fn next(db: &Connection) -> Result<i64, SessionError> {
    db.query_row(
        "SELECT next_operation FROM totp_control WHERE id=1",
        [],
        |r| r.get(0),
    )
    .map_err(required_row)
}

pub(in crate::session) fn reserve(db: &Connection) -> Result<i64, SessionError> {
    let next = next(db)?;
    let following = next.checked_add(1).ok_or(SessionError::Capacity)?;
    db.execute(
        "UPDATE totp_control SET next_operation=?1 WHERE id=1",
        [following],
    )
    .map_err(unavailable)?;
    Ok(next)
}

type FactorRow = (
    String,
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

pub(in crate::session) fn factor(db: &Connection) -> Result<Option<Factor>, SessionError> {
    let raw: Option<FactorRow> = db
        .query_row(
            "SELECT CASE WHEN octet_length(seed_ref)<=64 THEN seed_ref END,
         CASE WHEN octet_length(seed_digest)=64 THEN seed_digest END,last_step,
         CASE WHEN octet_length(consumption_json)<=2048 THEN consumption_json END,
         CASE WHEN octet_length(enrollment_json)<=2048 THEN enrollment_json END,
         CASE WHEN octet_length(intent_receipt)<=8192 THEN intent_receipt END,
         CASE WHEN octet_length(outcome_receipt)<=8192 THEN outcome_receipt END,confirmed_at
         FROM totp_factor WHERE id=1",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .optional()
        .map_err(unavailable)?;
    let Some((seed_ref, seed_digest, last_step, consumed, encoded, intent, outcome, activated)) =
        raw
    else {
        return Ok(None);
    };
    let enrollment = match (encoded, intent, outcome, activated) {
        (None, None, None, None) => None,
        (Some(encoded), Some(intent_receipt), Some(outcome_receipt), Some(activated_at)) => {
            Some(Proof {
                operation: operation(encoded)?,
                intent_receipt,
                outcome_receipt,
                activated_at,
            })
        }
        _ => return Err(SessionError::CorruptStore),
    };
    Ok(Some(Factor {
        seed_ref,
        seed_digest,
        last_step,
        consumption: operation(consumed)?,
        enrollment,
    }))
}

pub(in crate::session) fn pending(db: &Connection) -> Result<Option<Pending>, SessionError> {
    let raw: Option<(String, Option<String>, Option<String>)> = db
        .query_row(
            "SELECT CASE WHEN octet_length(operation_json)<=2048 THEN operation_json END,
         CASE WHEN octet_length(intent_receipt)<=8192 THEN intent_receipt END,
         CASE WHEN octet_length(outcome_receipt)<=8192 THEN outcome_receipt END
         FROM totp_pending WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(unavailable)?;
    raw.map(|(encoded, intent_receipt, outcome_receipt)| {
        Ok(Pending {
            operation: operation(encoded)?,
            intent_receipt,
            outcome_receipt,
        })
    })
    .transpose()
}

fn grant_row(row: &rusqlite::Row<'_>) -> Result<(String, String, String, i64), rusqlite::Error> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn proof(raw: (String, String, String, i64)) -> Result<Proof, SessionError> {
    Ok(Proof {
        operation: operation(raw.0)?,
        intent_receipt: raw.1,
        outcome_receipt: raw.2,
        activated_at: raw.3,
    })
}

const GRANTS: &str = "SELECT CASE WHEN octet_length(operation_json)<=2048 THEN operation_json END,
 CASE WHEN octet_length(intent_receipt)<=8192 THEN intent_receipt END,
 CASE WHEN octet_length(outcome_receipt)<=8192 THEN outcome_receipt END,activated_at FROM totp_grants";

pub(in crate::session) fn grants(db: &Connection) -> Result<Vec<Proof>, SessionError> {
    let mut statement = db
        .prepare(&format!("{GRANTS} LIMIT 1025"))
        .map_err(unavailable)?;
    statement
        .query_map([], grant_row)
        .map_err(unavailable)?
        .map(|r| proof(r.map_err(unavailable)?))
        .collect()
}

pub(in crate::session) fn grant(
    db: &Connection,
    session: &str,
) -> Result<Option<Proof>, SessionError> {
    db.query_row(
        &format!("{GRANTS} WHERE session_id=?1"),
        [session],
        grant_row,
    )
    .optional()
    .map_err(unavailable)?
    .map(proof)
    .transpose()
}

pub(in crate::session) fn consume(
    db: &Connection,
    operation: &Operation,
) -> Result<(), SessionError> {
    if matches!(operation.kind, super::model::Kind::Recovered { .. }) {
        return Err(SessionError::CorruptStore);
    }
    match factor(db)? {
        None => {
            db.execute("INSERT INTO totp_factor(id,seed_ref,seed_digest,last_step,consumption_json) VALUES(1,?1,?2,?3,?4)",
                params![operation.seed_ref,operation.seed_digest,operation.step,operation.encoded()?]).map_err(unavailable)?;
        }
        Some(factor) => {
            if factor.last_step >= operation.step
                || factor.seed_ref != operation.seed_ref
                || factor.seed_digest != operation.seed_digest
            {
                return Err(SessionError::CorruptStore);
            }
            db.execute(
                "UPDATE totp_factor SET last_step=?1,consumption_json=?2 WHERE id=1",
                params![operation.step, operation.encoded()?],
            )
            .map_err(unavailable)?;
        }
    }
    db.execute(
        "INSERT INTO totp_pending(id,operation_json) VALUES(1,?1)",
        [operation.encoded()?],
    )
    .map_err(unavailable)?;
    Ok(())
}

pub(in crate::session) fn activate(
    db: &Connection,
    pending: &Pending,
    now: i64,
) -> Result<(), SessionError> {
    let intent = pending
        .intent_receipt
        .as_ref()
        .ok_or(SessionError::AuditUnavailable)?;
    let outcome = pending
        .outcome_receipt
        .as_ref()
        .ok_or(SessionError::AuditUnavailable)?;
    let encoded = pending.operation.encoded()?;
    match pending.operation.kind {
        super::model::Kind::Enroll => {
            if db.execute("UPDATE totp_factor SET enrollment_json=?1,intent_receipt=?2,outcome_receipt=?3,confirmed_at=?4 WHERE id=1 AND enrollment_json IS NULL",
                params![encoded,intent,outcome,now]).map_err(unavailable)? != 1 {
                return Err(SessionError::CorruptStore);
            }
        }
        super::model::Kind::StepUp => {
            let id = pending.operation.session_id.as_ref().ok_or(SessionError::CorruptStore)?;
            db.execute("INSERT INTO totp_grants(session_id,operation_json,intent_receipt,outcome_receipt,activated_at) VALUES(?1,?2,?3,?4,?5)
                ON CONFLICT(session_id) DO UPDATE SET operation_json=excluded.operation_json,intent_receipt=excluded.intent_receipt,outcome_receipt=excluded.outcome_receipt,activated_at=excluded.activated_at",
                params![id,encoded,intent,outcome,now]).map_err(unavailable)?;
        }
        super::model::Kind::Recovered { .. } => return Err(SessionError::CorruptStore),
    }
    db.execute("DELETE FROM totp_pending WHERE id=1", [])
        .map_err(unavailable)?;
    Ok(())
}

/// 调用方在同一事务内验证恢复许可和原绑定；此处仅执行真正的因子替换。
pub(in crate::session) fn replace(db: &Connection, proof: &Proof) -> Result<(), SessionError> {
    if db.is_autocommit() {
        return Err(SessionError::StorageUnavailable);
    }
    if !matches!(proof.operation.kind, super::model::Kind::Recovered { operation } if operation > 0)
    {
        return Err(SessionError::CorruptStore);
    }
    if pending(db)?.is_some() {
        return Err(SessionError::OperationPending);
    }
    let operation = &proof.operation;
    let encoded = operation.encoded()?;
    if db
        .execute(
            "UPDATE totp_factor SET seed_ref=?1,seed_digest=?2,last_step=?3,consumption_json=?4,
         enrollment_json=?4,intent_receipt=?5,outcome_receipt=?6,confirmed_at=?7 WHERE id=1",
            params![
                operation.seed_ref,
                operation.seed_digest,
                operation.step,
                encoded,
                proof.intent_receipt,
                proof.outcome_receipt,
                proof.activated_at
            ],
        )
        .map_err(unavailable)?
        != 1
    {
        return Err(SessionError::CorruptStore);
    }
    // 旧证明不再可用于 L3；控制序号、限流桶及恢复高水位均不重置。
    db.execute("DELETE FROM totp_grants", [])
        .map_err(unavailable)?;
    Ok(())
}
