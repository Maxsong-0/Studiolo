//! v1 schema 只允许精确的两张表及 SQLite 生成的唯一索引，不自动接纳迁移。

use super::unavailable;
use crate::{AuditError, IntegrityHash, codec::CODEC};
use rusqlite::{Connection, params};

const APPLICATION_ID: i64 = 0x5354_4155;
const MAX_SCHEMA_SQL_BYTES: i64 = 1024;
const META: &str = "CREATE TABLE audit_meta (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    schema_version INTEGER NOT NULL,
    codec TEXT NOT NULL,
    last_offset INTEGER NOT NULL CHECK(last_offset >= 0),
    last_hash TEXT NOT NULL
) STRICT";
const EVENTS: &str = "CREATE TABLE audit_events (
    offset INTEGER PRIMARY KEY CHECK(offset > 0),
    event_id TEXT NOT NULL,
    phase TEXT NOT NULL CHECK(phase IN ('intent', 'outcome')),
    record_json TEXT NOT NULL,
    UNIQUE(event_id, phase)
) STRICT";

pub(super) fn initialize(connection: &Connection, created: bool) -> Result<(), AuditError> {
    let objects: i64 = connection
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
        .map_err(unavailable)?;
    if objects == 0 && created {
        connection
            .execute_batch(&format!(
                "{META}; {EVENTS}; PRAGMA user_version=1; PRAGMA application_id={APPLICATION_ID};"
            ))
            .map_err(unavailable)?;
        connection
            .execute(
                "INSERT INTO audit_meta VALUES(1, 1, ?1, 0, ?2)",
                params![CODEC, IntegrityHash::genesis().to_hex()],
            )
            .map_err(unavailable)?;
    }
    validate(connection)
}

pub(super) fn validate(connection: &Connection) -> Result<(), AuditError> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(unavailable)?;
    let application: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(unavailable)?;
    if version != 1 || application != APPLICATION_ID {
        return Err(AuditError::UnsupportedSchema);
    }
    // 先拒绝额外对象及超长声明，再在同一事务快照内读取最多三条有界文本。
    // SQLite 自身加载 schema 所需的内存不属于这里的应用结果集预算。
    let count: i64 = connection
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
        .map_err(unavailable)?;
    if count != 3 {
        return Err(AuditError::CorruptStore);
    }
    let oversized: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema
         WHERE typeof(type) != 'text' OR octet_length(type) NOT BETWEEN 1 AND 5
            OR typeof(name) != 'text' OR octet_length(name) NOT BETWEEN 1 AND 64
            OR (sql IS NOT NULL AND (typeof(sql) != 'text'
                OR octet_length(sql) NOT BETWEEN 1 AND ?1))",
            params![MAX_SCHEMA_SQL_BYTES],
            |row| row.get(0),
        )
        .map_err(unavailable)?;
    if oversized != 0 {
        return Err(AuditError::CorruptStore);
    }
    let mut statement = connection
        .prepare("SELECT type, name, sql FROM sqlite_schema ORDER BY name")
        .map_err(unavailable)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(unavailable)?;
    let actual = rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)?;
    let mut expected = vec![
        (
            "table".to_owned(),
            "audit_meta".to_owned(),
            Some(META.to_owned()),
        ),
        (
            "table".to_owned(),
            "audit_events".to_owned(),
            Some(EVENTS.to_owned()),
        ),
        (
            "index".to_owned(),
            "sqlite_autoindex_audit_events_1".to_owned(),
            None,
        ),
    ];
    expected.sort_by(|left, right| left.1.cmp(&right.1));
    if actual != expected {
        return Err(AuditError::CorruptStore);
    }
    // 任一错误即失败，不收集与损坏行数一起增长的诊断列表。
    let mut check = connection
        .prepare("PRAGMA quick_check(1)")
        .map_err(unavailable)?;
    let checks = check
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(unavailable)?;
    if checks != ["ok"] {
        return Err(AuditError::CorruptStore);
    }
    Ok(())
}

pub(super) fn head(connection: &Connection) -> Result<(u64, String), AuditError> {
    let count: i64 = connection
        .query_row("SELECT count(*) FROM audit_meta", [], |row| row.get(0))
        .map_err(unavailable)?;
    if count != 1 {
        return Err(AuditError::CorruptStore);
    }
    let (version, codec, offset, hash): (i64, Option<String>, i64, Option<String>) = connection
        .query_row(
            "SELECT schema_version,
            CASE WHEN octet_length(codec) = ?1 THEN codec END,
            last_offset,
            CASE WHEN octet_length(last_hash) = 64 THEN last_hash END
         FROM audit_meta WHERE id=1",
            params![CODEC.len() as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(unavailable)?;
    if version != 1 || codec.as_deref() != Some(CODEC) {
        return Err(AuditError::UnsupportedSchema);
    }
    let hash = hash.ok_or(AuditError::CorruptStore)?;
    Ok((
        u64::try_from(offset).map_err(|_| AuditError::CorruptStore)?,
        hash,
    ))
}
