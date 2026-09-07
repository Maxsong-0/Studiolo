//! 冻结原始审计行的摘要；跨库确认只使用这条原事件与原回执。

use super::{SessionError, valid_hash};
use crate::session::{database::unavailable, validation};
use rusqlite::{Connection, OptionalExtension, Row, types::ValueRef};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use studiolo_audit_ledger::AuditEvent;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Proof {
    pub sequence: i64,
    pub seal: String,
}

pub(super) struct Original {
    pub event: AuditEvent,
    pub receipt: String,
}

const SELECT: &str = "SELECT sequence,event_id,phase,event_json,event_digest,kind,bucket,through_count,receipt_json FROM audit_outbox WHERE sequence=?1";

pub(super) fn for_event(
    db: &Connection,
    id: &str,
    phase: &str,
) -> Result<Option<Proof>, SessionError> {
    let sequence: i64 = db
        .query_row(
            "SELECT sequence FROM audit_outbox WHERE event_id=?1 AND phase=?2",
            rusqlite::params![id, phase],
            |row| row.get(0),
        )
        .map_err(unavailable)?;
    capture(db, sequence)
}

fn capture(db: &Connection, sequence: i64) -> Result<Option<Proof>, SessionError> {
    db.query_row(SELECT, [sequence], |row| {
        let receipt: Option<String> = row.get(8)?;
        Ok(if receipt.is_some() {
            Some(Proof {
                sequence,
                seal: seal_row(row, 9, b"auth-retention-event-v1\0")?,
            })
        } else {
            None
        })
    })
    .map_err(unavailable)
}

pub(super) fn load(db: &Connection, expected: &Proof) -> Result<Original, SessionError> {
    if expected.sequence < 1 || !valid_hash(&expected.seal) {
        return Err(SessionError::CorruptStore);
    }
    let raw: Option<(String, Option<String>, String)> = db
        .query_row(SELECT, [expected.sequence], |row| {
            Ok((
                row.get(3)?,
                row.get(8)?,
                seal_row(row, 9, b"auth-retention-event-v1\0")?,
            ))
        })
        .optional()
        .map_err(unavailable)?;
    let Some((encoded, receipt, seal)) = raw else {
        return Err(SessionError::CorruptStore);
    };
    if seal != expected.seal {
        return Err(SessionError::CorruptStore);
    }
    let event: AuditEvent = super::decode(&encoded)?;
    let receipt = receipt.ok_or(SessionError::CorruptStore)?;
    validation::check_receipt(&receipt, &event)?;
    Ok(Original { event, receipt })
}

pub(super) fn seal_row(row: &Row<'_>, count: usize, domain: &[u8]) -> rusqlite::Result<String> {
    let mut hash = Sha256::new();
    hash.update(domain);
    for index in 0..count {
        // 类型和长度参与编码，NULL、文本和摘要字节不能混同。
        match row.get_ref(index)? {
            ValueRef::Null => hash.update([0]),
            ValueRef::Integer(value) => {
                hash.update([1]);
                hash.update(value.to_be_bytes());
            }
            ValueRef::Real(value) => {
                hash.update([2]);
                hash.update(value.to_bits().to_be_bytes());
            }
            ValueRef::Text(value) => {
                hash.update([3]);
                hash.update((value.len() as u64).to_be_bytes());
                hash.update(value);
            }
            ValueRef::Blob(value) => {
                hash.update([4]);
                hash.update((value.len() as u64).to_be_bytes());
                hash.update(value);
            }
        }
    }
    Ok(crate::session::secrets::hex(&hash.finalize()))
}
