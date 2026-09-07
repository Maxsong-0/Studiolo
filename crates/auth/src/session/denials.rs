//! 拒绝计数与安全事件分区；每种原因最多一个不可变待交付快照。

use super::{SessionError, database::unavailable, events, outbox};
use rusqlite::{Connection, params};
use serde_json::json;
use studiolo_audit_ledger::AuditPhase;

#[derive(Clone, Copy)]
pub(super) enum Denial {
    Identity = 0,
    AccessExpired = 1,
    Unknown = 2,
    Csrf = 3,
    Expired = 4,
    Clock = 5,
    Issue = 6,
    Capacity = 7,
}

impl Denial {
    fn label(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::AccessExpired => "access_expired",
            Self::Unknown => "unknown_session",
            Self::Csrf => "csrf",
            Self::Expired => "expired",
            Self::Clock => "clock",
            Self::Issue => "issue",
            Self::Capacity => "capacity",
        }
    }
}

pub(super) fn record(db: &Connection, denial: Denial) -> Result<(), SessionError> {
    db.execute("UPDATE auth_denials SET total=CASE WHEN total<9223372036854775807 THEN total+1 ELSE total END WHERE kind=?1", [denial as i64]).map_err(unavailable)?;
    schedule(db, denial)
}

pub(super) fn schedule(db: &Connection, denial: Denial) -> Result<(), SessionError> {
    let kind = denial as i64;
    let pending: i64 = db
        .query_row(
            "SELECT count(*) FROM audit_outbox WHERE bucket=?1",
            [kind],
            |r| r.get(0),
        )
        .map_err(unavailable)?;
    if pending != 0 {
        return Ok(());
    }
    let (total, delivered): (i64, i64) = db
        .query_row(
            "SELECT total,delivered FROM auth_denials WHERE kind=?1",
            [kind],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(unavailable)?;
    if total == delivered {
        return Ok(());
    }
    let event = events::event(
        &format!("{}:denied:{kind}:{total}", super::schema::store_id(db)?),
        AuditPhase::Outcome,
        "session.denied",
        json!({"reason":denial.label(),"after_count":delivered,"through_count":total}),
    )?;
    outbox::enqueue(db, &event, "denied", Some((kind, total)))
}

pub(super) fn acknowledge(
    db: &Connection,
    kind: i64,
    through: i64,
    receipt: &str,
) -> Result<(), SessionError> {
    db.execute("UPDATE auth_denials SET delivered=?1,last_receipt=?2 WHERE kind=?3 AND delivered<?1 AND total>=?1",
        params![through,receipt,kind]).map_err(unavailable)?;
    let denial = match kind {
        0 => Denial::Identity,
        1 => Denial::AccessExpired,
        2 => Denial::Unknown,
        3 => Denial::Csrf,
        4 => Denial::Expired,
        5 => Denial::Clock,
        6 => Denial::Issue,
        7 => Denial::Capacity,
        _ => return Err(SessionError::CorruptStore),
    };
    schedule(db, denial)
}
