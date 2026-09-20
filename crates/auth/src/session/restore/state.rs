//! 静止恢复副本的有界快照、证明集合和原子撤权；不观察或重置安全时钟。

use super::{RestoreInvalidation, audit};
use crate::session::{SessionError, database::unavailable, recovery, retention, schema, totp};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use studiolo_audit_ledger::AuditEvent;

pub(super) struct Snapshot {
    seal: String,
    epoch: i64,
    pub originals: Vec<(AuditEvent, String)>,
}

pub(super) fn capture(db: &Connection) -> Result<Snapshot, SessionError> {
    if schema::version(db)? != 4 {
        return Err(SessionError::MigrationRequired);
    }
    if totp::store::factor(db)?.is_none_or(|factor| factor.enrollment.is_none()) {
        return Err(SessionError::FactorUnavailable);
    }
    let pending: i64 = db.query_row(
        "SELECT (SELECT count(*) FROM totp_pending)+(SELECT count(*) FROM recovery_pending)+(SELECT count(*) FROM auth_retention_plan)",
        [], |row| row.get(0)).map_err(unavailable)?;
    if pending != 0 {
        return Err(SessionError::OperationPending);
    }
    let (epoch, next): (i64, i64) = db
        .query_row(
            "SELECT epoch,next_sequence FROM auth_meta WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(unavailable)?;
    epoch.checked_add(1).ok_or(SessionError::Capacity)?;
    next.checked_add(2).ok_or(SessionError::Capacity)?;
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM audit_outbox WHERE bucket IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(unavailable)?;
    if count > schema::MAX_SECURITY_EVENTS - 2 {
        return Err(SessionError::Capacity);
    }
    let originals = recovery::restore_proofs(db, &schema::store_id(db)?)?;
    Ok(Snapshot {
        seal: seal(db)?,
        epoch,
        originals,
    })
}

// Database::transaction 已逐字验证表闭集、类型和大小预算。对全部原表作带类型的
// 摘要，防止解锁重确认审计期间的并发变动混进已确认快照；摘要永不对外输出。
fn seal(db: &Connection) -> Result<String, SessionError> {
    let mut query = db
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .map_err(unavailable)?;
    let names = query
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(unavailable)?;
    let mut hash = Sha256::new();
    hash.update(b"auth-restore-snapshot-v1\0");
    for name in names {
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        let mut statement = db
            .prepare(&format!("SELECT * FROM {name} ORDER BY rowid"))
            .map_err(unavailable)?;
        let count = statement.column_count();
        let mut rows = statement.query([]).map_err(unavailable)?;
        while let Some(row) = rows.next().map_err(unavailable)? {
            hash.update(
                retention::seal_row(row, count, b"auth-restore-row-v1\0")
                    .map_err(unavailable)?
                    .as_bytes(),
            );
        }
        hash.update([0]);
    }
    Ok(crate::session::secrets::hex(&hash.finalize()))
}

pub(super) fn commit(
    db: &Connection,
    restore_id: &str,
    frozen: &Snapshot,
) -> Result<RestoreInvalidation, SessionError> {
    let current = capture(db)?;
    if current.seal != frozen.seal {
        return Err(SessionError::OperationPending);
    }
    let epoch = frozen.epoch.checked_add(1).ok_or(SessionError::Capacity)?;
    let revoked_sessions = db
        .execute(
            "UPDATE sessions SET state='revoked' WHERE state IN ('active','pending')",
            [],
        )
        .map_err(unavailable)?;
    db.execute("UPDATE auth_meta SET epoch=?1 WHERE id=1", [epoch])
        .map_err(unavailable)?;
    let cleared_l3 = db
        .execute("DELETE FROM totp_grants", [])
        .map_err(unavailable)?;
    let invalidated_recovery_codes = db
        .execute("DELETE FROM recovery_codes", [])
        .map_err(unavailable)?;
    db.execute(
        "UPDATE recovery_control SET current_json=NULL,used_mask=0 WHERE id=1",
        [],
    )
    .map_err(unavailable)?;
    let report = RestoreInvalidation {
        epoch,
        revoked_sessions,
        cleared_l3,
        invalidated_recovery_codes,
        audit_confirmed: false,
    };
    audit::record(db, restore_id, &report)?;
    let config: String = db
        .query_row("SELECT config FROM auth_meta WHERE id=1", [], |row| {
            row.get(0)
        })
        .map_err(unavailable)?;
    schema::validate(db, &config)?;
    Ok(report)
}
