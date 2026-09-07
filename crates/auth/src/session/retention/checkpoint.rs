//! 已提交的清理许可锚点；每次覆盖，不在 Auth 中累计历史检查点。

use super::{SessionError, current_epoch, decode, encode, valid_hash};
use crate::session::{database::unavailable, events, schema, validation};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use studiolo_audit_ledger::{AuditEvent, AuditPhase};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Checkpoint {
    pub sequence: i64,
    pub previous_sequence: i64,
    pub previous_manifest: String,
    pub epoch_base: i64,
    pub issuance_floor: i64,
    pub event_sequence_barrier: i64,
    pub manifest_hash: String,
    pub reclaimed_sessions: usize,
}

impl Checkpoint {
    pub fn genesis() -> Self {
        Self {
            sequence: 0,
            previous_sequence: 0,
            previous_manifest: "0".repeat(64),
            epoch_base: 1,
            issuance_floor: 0,
            event_sequence_barrier: 0,
            manifest_hash: "0".repeat(64),
            reclaimed_sessions: 0,
        }
    }

    pub fn event(&self, store: &str) -> Result<AuditEvent, SessionError> {
        events::event(
            &format!("{store}:compact:{}", self.sequence),
            AuditPhase::Outcome,
            "session.compacted",
            json!({
                "manifest_hash":self.manifest_hash,"reclaimed_sessions":self.reclaimed_sessions,
                "epoch_base":self.epoch_base,"issuance_floor":self.issuance_floor,
                "event_sequence_barrier":self.event_sequence_barrier,
                "previous_sequence":self.previous_sequence,"previous_manifest":self.previous_manifest,
                "cleanup_authorized":true
            }),
        )
    }
}

pub(super) fn read(db: &Connection) -> Result<Checkpoint, SessionError> {
    if schema::version(db)? == 1 {
        return Ok(Checkpoint::genesis());
    }
    let encoded: Option<String> = db.query_row(
        "SELECT CASE WHEN octet_length(checkpoint_json) BETWEEN 1 AND 4096 THEN checkpoint_json END FROM auth_retention_checkpoint WHERE id=1",
        [], |row| row.get(0)).map_err(unavailable)?;
    decode(&encoded.ok_or(SessionError::CorruptStore)?)
}

pub(super) fn validate(db: &Connection) -> Result<(), SessionError> {
    let value = read(db)?;
    let receipt: Option<String> = db.query_row(
        "SELECT CASE WHEN receipt_json IS NULL OR octet_length(receipt_json) BETWEEN 1 AND 8192 THEN receipt_json ELSE '' END FROM auth_retention_checkpoint WHERE id=1",
        [], |row| row.get(0)).map_err(unavailable)?;
    if value.sequence == 0 {
        if value != Checkpoint::genesis() || receipt.is_some() {
            return Err(SessionError::CorruptStore);
        }
        return Ok(());
    }
    let next: i64 = db
        .query_row("SELECT next_sequence FROM auth_meta WHERE id=1", [], |r| {
            r.get(0)
        })
        .map_err(unavailable)?;
    let next_issuance: i64 = db
        .query_row(
            "SELECT next_issuance FROM credential_state WHERE id=1",
            [],
            |r| r.get(0),
        )
        .map_err(unavailable)?;
    let minimum: Option<i64> = db
        .query_row("SELECT min(epoch) FROM sessions", [], |r| r.get(0))
        .map_err(unavailable)?;
    if value.sequence <= value.previous_sequence
        || value.previous_sequence < 0
        || value.sequence >= next
        || value.event_sequence_barrier != value.sequence - 1
        || value.epoch_base < 1
        || value.epoch_base > minimum.unwrap_or(current_epoch(db)?)
        || value.issuance_floor < 1
        || value.issuance_floor > next_issuance
        || value.reclaimed_sessions > super::BATCH
        || !valid_hash(&value.manifest_hash)
        || !valid_hash(&value.previous_manifest)
    {
        return Err(SessionError::CorruptStore);
    }
    let event = value.event(&schema::store_id(db)?)?;
    validation::check_receipt(&receipt.ok_or(SessionError::CorruptStore)?, &event)
}

pub(super) fn replace(
    db: &Connection,
    value: &Checkpoint,
    receipt: &str,
) -> Result<(), SessionError> {
    db.execute(
        "UPDATE auth_retention_checkpoint SET checkpoint_json=?1,receipt_json=?2 WHERE id=1",
        rusqlite::params![encode(value)?, receipt],
    )
    .map_err(unavailable)?;
    Ok(())
}
