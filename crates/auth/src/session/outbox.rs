//! 跨库交付：只在 Auth 事务外调用审计，回执失败时精确重放原事件。

use super::{SessionError, SessionManager, database::unavailable, denials, events, schema};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use studiolo_audit_ledger::{AuditEvent, AuditPhase, AuditRecord};

struct Pending {
    sequence: i64,
    event: AuditEvent,
    encoded: String,
    bucket: Option<i64>,
    through: Option<i64>,
}

type PendingRow = (i64, String, String, String, Option<i64>, Option<i64>);

pub(super) fn enqueue(
    db: &Connection,
    event: &AuditEvent,
    kind: &str,
    bucket: Option<(i64, i64)>,
) -> Result<(), SessionError> {
    if bucket.is_none() {
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM audit_outbox WHERE bucket IS NULL",
                [],
                |r| r.get(0),
            )
            .map_err(unavailable)?;
        if count >= schema::MAX_SECURITY_EVENTS {
            return Err(SessionError::Capacity);
        }
    }
    let encoded = events::encode(event)?;
    let sequence = schema::next_sequence(db)?;
    db.execute("INSERT INTO audit_outbox(sequence,event_id,phase,event_json,event_digest,kind,bucket,through_count) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![sequence,event.event_id().as_str(),events::phase(event.phase()),encoded,
            super::secrets::hex(&Sha256::digest(encoded.as_bytes())),kind,bucket.map(|v|v.0),bucket.map(|v|v.1)]).map_err(unavailable)?;
    Ok(())
}

fn pending(db: &Connection) -> Result<Option<Pending>, SessionError> {
    let raw: Option<PendingRow> = db.query_row(
        "SELECT sequence,event_id,phase,event_json,bucket,through_count FROM audit_outbox WHERE receipt_json IS NULL ORDER BY sequence LIMIT 1", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(unavailable)?;
    let Some((sequence, id, phase, encoded, bucket, through)) = raw else {
        return Ok(None);
    };
    let event: AuditEvent =
        serde_json::from_str(&encoded).map_err(|_| SessionError::CorruptStore)?;
    if event.event_id().as_str() != id
        || events::phase(event.phase()) != phase
        || events::encode(&event)? != encoded
        || !matches!(
            event.action(),
            "session.issue" | "session.revoked" | "session.expired" | "session.denied"
        )
    {
        return Err(SessionError::CorruptStore);
    }
    Ok(Some(Pending {
        sequence,
        event,
        encoded,
        bucket,
        through,
    }))
}

pub(super) fn receipt(record: &AuditRecord, event: &AuditEvent) -> Result<String, SessionError> {
    if record.event() != event || record.offset() == 0 {
        return Err(SessionError::AuditUnavailable);
    }
    record
        .verify_integrity()
        .map_err(|_| SessionError::AuditUnavailable)?;
    let encoded = serde_json::to_string(record).map_err(|_| SessionError::AuditUnavailable)?;
    if encoded.len() > schema::MAX_EVENT_BYTES as usize {
        return Err(SessionError::AuditUnavailable);
    }
    let decoded: AuditRecord =
        serde_json::from_str(&encoded).map_err(|_| SessionError::AuditUnavailable)?;
    if &decoded != record {
        return Err(SessionError::AuditUnavailable);
    }
    Ok(encoded)
}

pub(super) fn validate(db: &Connection) -> Result<(), SessionError> {
    super::validation::outbox(db)
}

impl SessionManager {
    /// 顺序补记最多 256 条原事件，不重新授权、不激活 pending、不重做吊销。
    pub fn flush_audit(&self, max_events: usize) -> Result<usize, SessionError> {
        if !(1..=256).contains(&max_events) {
            return Err(SessionError::Configuration);
        }
        let mut delivered = 0;
        for _ in 0..max_events {
            let Some(pending) = self.database.transaction(pending)? else {
                break;
            };
            let record = self
                .audit
                .append(pending.event.clone())
                .map_err(|_| SessionError::AuditUnavailable)?;
            let receipt = receipt(&record, &pending.event)?;
            self.database
                .transaction(|db| acknowledge(db, &pending, &receipt))?;
            delivered += 1;
        }
        Ok(delivered)
    }
}

fn acknowledge(db: &Connection, pending: &Pending, receipt: &str) -> Result<(), SessionError> {
    let current: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT event_json,receipt_json FROM audit_outbox WHERE sequence=?1",
            [pending.sequence],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(unavailable)?;
    let Some((encoded, existing)) = current else {
        // 聚合拒绝或安全回收可能先被另一个实例确认；事务已验证完整引用与锚点。
        // 安全事件仅允许落在已提交回收屏障之前，且绝不补写已删除的会话。
        return if pending.bucket.is_some()
            || pending.sequence <= super::retention::event_sequence_barrier(db)?
        {
            Ok(())
        } else {
            Err(SessionError::CorruptStore)
        };
    };
    if encoded != pending.encoded || existing.as_ref().is_some_and(|old| old != receipt) {
        return Err(SessionError::CorruptStore);
    }
    if existing.is_some() {
        return Ok(());
    }
    if pending.event.action() == "session.issue" && pending.event.phase() == AuditPhase::Outcome {
        db.execute(
            "UPDATE sessions SET grant_receipt=?1 WHERE grant_id=?2 AND grant_receipt IS NULL",
            params![receipt, pending.event.event_id().as_str()],
        )
        .map_err(unavailable)?;
    }
    if let Some(bucket) = pending.bucket {
        db.execute(
            "DELETE FROM audit_outbox WHERE sequence=?1",
            [pending.sequence],
        )
        .map_err(unavailable)?;
        denials::acknowledge(
            db,
            bucket,
            pending.through.ok_or(SessionError::CorruptStore)?,
            receipt,
        )?;
    } else {
        db.execute(
            "UPDATE audit_outbox SET receipt_json=?1 WHERE sequence=?2",
            params![receipt, pending.sequence],
        )
        .map_err(unavailable)?;
    }
    Ok(())
}
