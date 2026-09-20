//! 配对恢复事件永久留在原 outbox；补记不重新执行撤权。

use super::{RestoreInvalidation, valid_id};
use crate::session::{SessionError, SessionManager, database::unavailable, events, outbox, schema};
use rusqlite::{Connection, params};
use serde_json::json;
use studiolo_audit_ledger::{AuditEvent, AuditPhase};

pub(super) struct Prior {
    pub report: RestoreInvalidation,
    pub events: Vec<(AuditEvent, Option<String>)>,
}

pub(in crate::session) fn valid_payload(event: &AuditEvent, namespace: &str) -> bool {
    let value = event.payload().as_value();
    value.as_object().is_some_and(|object| object.len() == 8)
        && event.action() == "session.revoked"
        && value["scope"] == "restore"
        && value["effective"] == "already_revoked"
        && value["version"].as_u64() == Some(1)
        && value["identity_version"].as_i64().is_some_and(|v| v > 1)
        && value["revoked"].as_u64().is_some_and(|v| v <= 1024)
        && value["cleared_l3"].as_u64().is_some_and(|v| v <= 1024)
        && value["invalidated_recovery_code_count"]
            .as_u64()
            .is_some_and(|v| matches!(v, 0 | 10))
        && value["target"].as_str().is_some_and(|id| {
            valid_id(id) && event.event_id().as_str() == format!("{namespace}restore:{id}")
        })
}

pub(super) fn record(
    db: &Connection,
    restore_id: &str,
    report: &RestoreInvalidation,
) -> Result<(), SessionError> {
    let id = format!("{}:restore:{restore_id}", schema::store_id(db)?);
    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
        let event = events::event(
            &id,
            phase,
            "session.revoked",
            json!({
                "scope":"restore", "target":restore_id, "revoked":report.revoked_sessions,
                "identity_version":report.epoch, "effective":"already_revoked",
                "cleared_l3":report.cleared_l3,
                "invalidated_recovery_code_count":report.invalidated_recovery_codes, "version":1
            }),
        )?;
        outbox::enqueue(db, &event, "revoke", None)?;
    }
    Ok(())
}

pub(super) fn read(db: &Connection, restore_id: &str) -> Result<Option<Prior>, SessionError> {
    let namespace = format!("{}:", schema::store_id(db)?);
    let id = format!("{namespace}restore:{restore_id}");
    let mut statement = db
        .prepare(
            "SELECT event_json,receipt_json FROM audit_outbox WHERE event_id=?1 ORDER BY sequence",
        )
        .map_err(unavailable)?;
    let raw = statement
        .query_map([id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(unavailable)?;
    if raw.is_empty() {
        return Ok(None);
    }
    if raw.len() != 2 {
        return Err(SessionError::CorruptStore);
    }
    let mut originals = Vec::new();
    for ((encoded, receipt), phase) in raw
        .into_iter()
        .zip([AuditPhase::Intent, AuditPhase::Outcome])
    {
        let event: AuditEvent =
            serde_json::from_str(&encoded).map_err(|_| SessionError::CorruptStore)?;
        if event.phase() != phase || !valid_payload(&event, &namespace) {
            return Err(SessionError::CorruptStore);
        }
        originals.push((event, receipt));
    }
    if originals[0].0.payload() != originals[1].0.payload() {
        return Err(SessionError::CorruptStore);
    }
    let value = originals[0].0.payload().as_value();
    let report = RestoreInvalidation {
        epoch: value["identity_version"]
            .as_i64()
            .ok_or(SessionError::CorruptStore)?,
        revoked_sessions: value["revoked"]
            .as_u64()
            .ok_or(SessionError::CorruptStore)? as usize,
        cleared_l3: value["cleared_l3"]
            .as_u64()
            .ok_or(SessionError::CorruptStore)? as usize,
        invalidated_recovery_codes: value["invalidated_recovery_code_count"]
            .as_u64()
            .ok_or(SessionError::CorruptStore)? as usize,
        audit_confirmed: false,
    };
    Ok(Some(Prior {
        report,
        events: originals,
    }))
}

pub(super) fn deliver(manager: &SessionManager, restore_id: &str) -> Result<bool, SessionError> {
    let prior = manager
        .database
        .transaction(|db| read(db, restore_id))?
        .ok_or(SessionError::CorruptStore)?;
    for (event, previous) in prior.events {
        let record = manager
            .audit
            .append(event.clone())
            .map_err(|_| SessionError::AuditUnavailable)?;
        let receipt = outbox::receipt(&record, &event)?;
        if previous.as_ref().is_some_and(|prior| prior != &receipt) {
            return Err(SessionError::AuditUnavailable);
        }
        manager.database.transaction(|db| {
            let current = read(db, restore_id)?.ok_or(SessionError::CorruptStore)?;
            let (_, stored) = current
                .events
                .iter()
                .find(|(item, _)| item == &event)
                .ok_or(SessionError::CorruptStore)?;
            if stored.as_ref().is_some_and(|prior| prior != &receipt) {
                return Err(SessionError::AuditUnavailable);
            }
            let changed = db
                .execute(
                    "UPDATE audit_outbox SET receipt_json=?1 WHERE event_id=?2 AND phase=?3",
                    params![
                        receipt,
                        event.event_id().as_str(),
                        events::phase(event.phase())
                    ],
                )
                .map_err(unavailable)?;
            if changed != 1 {
                return Err(SessionError::CorruptStore);
            }
            Ok(())
        })?;
    }
    Ok(true)
}

// 不改变 Auth 状态；只能把固定原事件交给同一账本重确认，不能接受另一合法偏移。
pub(super) fn confirm(
    manager: &SessionManager,
    proofs: &[(AuditEvent, String)],
) -> Result<(), SessionError> {
    for (event, expected) in proofs {
        let record = manager
            .audit
            .append(event.clone())
            .map_err(|_| SessionError::AuditUnavailable)?;
        if outbox::receipt(&record, event)? != *expected {
            return Err(SessionError::AuditUnavailable);
        }
    }
    Ok(())
}
