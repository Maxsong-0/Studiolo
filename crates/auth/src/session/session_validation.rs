//! 会话行、原始授权回执与终态证据的关联验证。

use super::{
    SessionError, SessionState,
    database::unavailable,
    events::{self, Grant},
    rows, schema,
    validation::check_receipt,
};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use studiolo_audit_ledger::AuditPhase;

pub(super) fn sessions(db: &Connection, config: &str) -> Result<(), SessionError> {
    let config: Value = serde_json::from_str(config).map_err(|_| SessionError::Configuration)?;
    let ttl = config["policy"]["absolute_seconds"]
        .as_i64()
        .ok_or(SessionError::Configuration)?
        * 1000;
    let floor: i64 = db
        .query_row("SELECT clock_floor FROM auth_meta", [], |r| r.get(0))
        .map_err(unavailable)?;
    let store_id = schema::store_id(db)?;
    let epoch: i64 = db
        .query_row("SELECT epoch FROM auth_meta", [], |r| r.get(0))
        .map_err(unavailable)?;
    epochs(db, &store_id, epoch)?;
    for row in rows::all(db)? {
        let access: i64 = db
            .query_row(
                "SELECT access_expiry FROM sessions WHERE id=?1",
                [row.info.id.as_str()],
                |r| r.get(0),
            )
            .map_err(unavailable)?;
        let grant = Grant {
            store_id: store_id.clone(),
            id: row.info.id.clone(),
            epoch: row.epoch,
            created: row.info.created_at_ms,
            absolute: row.info.absolute_expires_at_ms,
            access_expiry: access,
        };
        let expected = grant.event(AuditPhase::Outcome)?;
        if row.grant_id != expected.event_id().as_str()
            || row.info.last_seen_at_ms > floor
            || row
                .info
                .created_at_ms
                .checked_add(ttl)
                .is_none_or(|maximum| row.info.absolute_expires_at_ms > maximum)
        {
            return Err(SessionError::CorruptStore);
        }
        terminal(db, &row, &store_id, epoch)?;
        let paired:Option<(String,Option<String>)>=db.query_row("SELECT event_json,receipt_json FROM audit_outbox WHERE event_id=?1 AND phase='outcome'",[&row.grant_id],
            |r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(unavailable)?;
        let Some((event, paired_receipt)) = paired else {
            return Err(SessionError::CorruptStore);
        };
        if event != events::encode(&expected)? || row.grant_receipt != paired_receipt {
            return Err(SessionError::CorruptStore);
        }
        if let Some(receipt) = row.grant_receipt {
            check_receipt(&receipt, &expected)?;
        } else if row.info.state == SessionState::Active {
            return Err(SessionError::CorruptStore);
        }
    }
    Ok(())
}

fn terminal(
    db: &Connection,
    row: &rows::SessionRow,
    store_id: &str,
    epoch: i64,
) -> Result<(), SessionError> {
    let revoked: i64 = db
        .query_row(
            "SELECT count(*) FROM audit_outbox WHERE event_id=?1 AND phase='outcome'",
            [format!("{store_id}:revoke:{}", row.info.id.as_str())],
            |r| r.get(0),
        )
        .map_err(unavailable)?;
    let expired: Option<String> = db
        .query_row(
            "SELECT event_json FROM audit_outbox WHERE event_id=?1 AND phase='outcome'",
            [format!("{store_id}:expire:{}", row.info.id.as_str())],
            |r| r.get(0),
        )
        .optional()
        .map_err(unavailable)?;
    if row
        .activated_at
        .is_some_and(|at| at < row.info.created_at_ms || at > row.info.last_seen_at_ms)
    {
        return Err(SessionError::CorruptStore);
    }
    let valid = match row.info.state {
        SessionState::Active => {
            row.activated_at.is_some() && row.epoch == epoch && revoked == 0 && expired.is_none()
        }
        SessionState::Pending => {
            row.activated_at.is_none() && row.epoch == epoch && revoked == 0 && expired.is_none()
        }
        SessionState::Revoked => (revoked == 1 || row.epoch < epoch) && expired.is_none(),
        SessionState::Expired | SessionState::Abandoned => {
            if revoked != 0 {
                return Err(SessionError::CorruptStore);
            }
            if let Some(encoded) = expired {
                let event: studiolo_audit_ledger::AuditEvent =
                    serde_json::from_str(&encoded).map_err(|_| SessionError::CorruptStore)?;
                let expected = if row.info.state == SessionState::Expired {
                    "expired"
                } else {
                    "abandoned"
                };
                event.payload().as_value()["state"] == expected
                    && (row.info.state == SessionState::Expired) == row.activated_at.is_some()
            } else {
                false
            }
        }
    };
    if !valid {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}

fn epochs(db: &Connection, store_id: &str, epoch: i64) -> Result<(), SessionError> {
    let base = super::retention::epoch_base(db)?;
    if base < 1 || epoch < base || (schema::version(db)? == 1 && epoch > schema::MAX_SESSIONS + 1) {
        return Err(SessionError::CorruptStore);
    }
    let minimum: Option<i64> = db
        .query_row("SELECT min(epoch) FROM sessions", [], |row| row.get(0))
        .map_err(unavailable)?;
    if minimum.is_some_and(|minimum| base > minimum) {
        return Err(SessionError::CorruptStore);
    }
    let prefix = format!("{store_id}:revoke-all:");
    let mut statement = db
        .prepare("SELECT event_id FROM audit_outbox WHERE event_id LIKE ?1 AND phase='outcome'")
        .map_err(unavailable)?;
    let values = statement
        .query_map([format!("{prefix}%")], |r| r.get::<_, String>(0))
        .map_err(unavailable)?;
    let mut count = 0;
    for value in values {
        let value = value.map_err(unavailable)?;
        let value = value
            .strip_prefix(&prefix)
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or(SessionError::CorruptStore)?;
        if value <= base || value > epoch {
            return Err(SessionError::CorruptStore);
        }
        count += 1;
    }
    if count != epoch - base {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}
