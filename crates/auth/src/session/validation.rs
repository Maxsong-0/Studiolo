//! 有界记录之间的关联检查；哈希只能检测不一致损坏，不抵御同权限全库重写。

use super::{
    SessionError,
    database::{required_row, unavailable},
    event_validation::payload,
    events, schema, secrets,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use studiolo_audit_ledger::{AuditEvent, AuditRecord};

pub(super) fn outbox(db: &Connection) -> Result<(), SessionError> {
    let namespace = format!("{}:", schema::store_id(db)?);
    let mut statement=db.prepare("SELECT event_id,phase,event_json,event_digest,kind,bucket,through_count,receipt_json FROM audit_outbox ORDER BY sequence").map_err(unavailable)?;
    let mut rows = statement.query([]).map_err(unavailable)?;
    while let Some(row) = rows.next().map_err(unavailable)? {
        let id: String = row.get(0).map_err(unavailable)?;
        let phase: String = row.get(1).map_err(unavailable)?;
        let encoded: String = row.get(2).map_err(unavailable)?;
        let checksum: String = row.get(3).map_err(unavailable)?;
        let kind: String = row.get(4).map_err(unavailable)?;
        let bucket: Option<i64> = row.get(5).map_err(unavailable)?;
        let through: Option<i64> = row.get(6).map_err(unavailable)?;
        let receipt: Option<String> = row.get(7).map_err(unavailable)?;
        let event: AuditEvent =
            serde_json::from_str(&encoded).map_err(|_| SessionError::CorruptStore)?;
        if !id.starts_with(&namespace)
            || id != event.event_id().as_str()
            || phase != events::phase(event.phase())
            || events::encode(&event)? != encoded
            || secrets::hex(&Sha256::digest(encoded.as_bytes())) != checksum
        {
            return Err(SessionError::CorruptStore);
        }
        payload(&event, &kind, bucket, through, &namespace)?;
        if kind == "revoke" {
            let opposite = if phase == "intent" {
                "outcome"
            } else {
                "intent"
            };
            // 吊销两阶段同事务成对写入、成对回收；另一半缺失即库损坏。
            let other: String = db
                .query_row(
                    "SELECT event_json FROM audit_outbox WHERE event_id=?1 AND phase=?2",
                    rusqlite::params![&id, opposite],
                    |r| r.get(0),
                )
                .map_err(required_row)?;
            let other: AuditEvent =
                serde_json::from_str(&other).map_err(|_| SessionError::CorruptStore)?;
            if other.payload() != event.payload() || other.action() != event.action() {
                return Err(SessionError::CorruptStore);
            }
        }
        if let Some(bucket) = bucket {
            // 八种拒绝计数行建库即写入且从不删除，分区快照必有对应计数行。
            let (total, delivered): (i64, i64) = db
                .query_row(
                    "SELECT total,delivered FROM auth_denials WHERE kind=?1",
                    [bucket],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(required_row)?;
            if through.is_none_or(|value| value > total)
                || event.payload().as_value()["after_count"].as_i64() != Some(delivered)
                || receipt.is_some()
            {
                return Err(SessionError::CorruptStore);
            }
        }
        if let Some(receipt) = receipt {
            check_receipt(&receipt, &event)?;
        }
    }
    let mut statement = db
        .prepare("SELECT kind,delivered,last_receipt FROM auth_denials")
        .map_err(unavailable)?;
    let mut rows = statement.query([]).map_err(unavailable)?;
    while let Some(row) = rows.next().map_err(unavailable)? {
        let kind: i64 = row.get(0).map_err(unavailable)?;
        let delivered: i64 = row.get(1).map_err(unavailable)?;
        let receipt: Option<String> = row.get(2).map_err(unavailable)?;
        if let Some(receipt) = receipt {
            let record: AuditRecord =
                serde_json::from_str(&receipt).map_err(|_| SessionError::CorruptStore)?;
            payload(
                record.event(),
                "denied",
                Some(kind),
                Some(delivered),
                &namespace,
            )?;
            check_receipt(&receipt, record.event())?;
        } else if delivered != 0 {
            return Err(SessionError::CorruptStore);
        }
    }
    Ok(())
}
pub(super) fn check_receipt(encoded: &str, event: &AuditEvent) -> Result<(), SessionError> {
    let record: AuditRecord =
        serde_json::from_str(encoded).map_err(|_| SessionError::CorruptStore)?;
    if record.event() != event
        || record.offset() == 0
        || serde_json::to_string(&record).map_err(|_| SessionError::CorruptStore)? != encoded
    {
        return Err(SessionError::CorruptStore);
    }
    record
        .verify_integrity()
        .map_err(|_| SessionError::CorruptStore)
}
