//! 兼容既有格式 0：精确接纳旧表与索引，不隐式迁移，不把缺表当成初始化请求。

use rusqlite::{Connection, params};

use super::{SupplyError, unavailable};

const TABLES: &[(&str, &str)] = &[
    (
        "agent_type",
        "CREATE TABLE agent_type (
        name TEXT PRIMARY KEY CHECK (length(name) BETWEEN 1 AND 32),
        token_sha256 TEXT NOT NULL UNIQUE CHECK (length(token_sha256) = 64),
        issued_at TEXT NOT NULL,
        revoked_at TEXT)",
    ),
    (
        "agent_instance",
        "CREATE TABLE agent_instance (
        id TEXT PRIMARY KEY,
        type_name TEXT NOT NULL REFERENCES agent_type(name),
        session_digest TEXT NOT NULL,
        registered_at TEXT NOT NULL,
        last_seen_at TEXT NOT NULL)",
    ),
    (
        "reads",
        "CREATE TABLE reads (
        seq INTEGER PRIMARY KEY,
        recorded_at_ms INTEGER NOT NULL,
        recorded_at TEXT NOT NULL,
        type_name TEXT NOT NULL,
        instance_id TEXT NOT NULL,
        path TEXT NOT NULL,
        revision TEXT NOT NULL,
        tool TEXT NOT NULL,
        ledger_offset INTEGER NOT NULL UNIQUE)",
    ),
    (
        "outage",
        "CREATE TABLE outage (
        id TEXT PRIMARY KEY,
        started_at TEXT NOT NULL,
        ended_at TEXT,
        denied_count INTEGER NOT NULL DEFAULT 0,
        reason TEXT NOT NULL)",
    ),
];

const INDEXES: &[(&str, &str)] = &[
    (
        "reads_by_type",
        "CREATE INDEX reads_by_type ON reads(type_name, recorded_at_ms)",
    ),
    (
        "reads_by_instance",
        "CREATE INDEX reads_by_instance ON reads(instance_id, recorded_at_ms)",
    ),
    (
        "reads_by_path",
        "CREATE INDEX reads_by_path ON reads(path, recorded_at_ms)",
    ),
    (
        "reads_by_time",
        "CREATE INDEX reads_by_time ON reads(recorded_at_ms)",
    ),
];

const AUTO_INDEXES: &[(&str, &str)] = &[
    ("sqlite_autoindex_agent_type_1", "agent_type"),
    ("sqlite_autoindex_agent_type_2", "agent_type"),
    ("sqlite_autoindex_agent_instance_1", "agent_instance"),
    ("sqlite_autoindex_reads_1", "reads"),
    ("sqlite_autoindex_outage_1", "outage"),
];

type SchemaObject = (String, String, String, Option<String>);

pub(super) fn initialize(db: &Connection, created: bool) -> Result<(), SupplyError> {
    header(db)?;
    if created {
        let objects: i64 = db
            .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
            .map_err(unavailable)?;
        if objects != 0 {
            return Err(SupplyError::CorruptStore);
        }
        for (_, sql) in TABLES.iter().chain(INDEXES.iter()) {
            db.execute_batch(sql).map_err(unavailable)?;
        }
    }
    validate(db)
}

pub(super) fn version(db: &Connection) -> Result<i64, SupplyError> {
    db.pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(unavailable)
}

fn header(db: &Connection) -> Result<(), SupplyError> {
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(unavailable)?;
    let application: i64 = db
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .map_err(unavailable)?;
    // 现行发布格式从未设置这两个字段；不静默改写真实格式 0 的数据库头。
    if !(0..=1).contains(&version) || application != 0 {
        return Err(SupplyError::CorruptStore);
    }
    Ok(())
}

fn validate(db: &Connection) -> Result<(), SupplyError> {
    header(db)?;
    let count: i64 = db
        .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
        .map_err(unavailable)?;
    let children = version(db)? == 1;
    if count
        != (TABLES.len() + INDEXES.len() + AUTO_INDEXES.len() + if children { 3 } else { 0 }) as i64
    {
        return Err(SupplyError::CorruptStore);
    }
    // 先限制每条 schema 文本，应用层不会按损坏声明的任意长度分配字符串。
    let invalid: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_schema
         WHERE typeof(type) != 'text' OR octet_length(type) NOT BETWEEN 1 AND 5
            OR typeof(name) != 'text' OR octet_length(name) NOT BETWEEN 1 AND 64
            OR typeof(tbl_name) != 'text' OR octet_length(tbl_name) NOT BETWEEN 1 AND 64
            OR (sql IS NOT NULL AND (typeof(sql) != 'text'
                OR octet_length(sql) NOT BETWEEN 1 AND ?1))",
            params![2048],
            |r| r.get(0),
        )
        .map_err(unavailable)?;
    if invalid != 0 {
        return Err(SupplyError::CorruptStore);
    }
    let mut statement = db
        .prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY name")
        .map_err(unavailable)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(unavailable)?;
    let mut actual = rows
        .collect::<Result<Vec<SchemaObject>, _>>()
        .map_err(unavailable)?;
    for row in &mut actual {
        row.3 = row.3.as_deref().map(normalize);
    }
    let mut expected: Vec<SchemaObject> = Vec::with_capacity(count as usize);
    for (name, sql) in TABLES {
        expected.push((
            "table".into(),
            (*name).into(),
            (*name).into(),
            Some(normalize(sql)),
        ));
    }
    for (name, sql) in INDEXES {
        expected.push((
            "index".into(),
            (*name).into(),
            "reads".into(),
            Some(normalize(sql)),
        ));
    }
    for (name, table) in AUTO_INDEXES {
        expected.push(("index".into(), (*name).into(), (*table).into(), None));
    }
    if children {
        expected.push((
            "table".into(),
            "instance_parent".into(),
            "instance_parent".into(),
            Some(normalize(PARENT_TABLE)),
        ));
        expected.push((
            "index".into(),
            "instance_parent_by_parent".into(),
            "instance_parent".into(),
            Some(normalize(PARENT_INDEX)),
        ));
        expected.push((
            "index".into(),
            "sqlite_autoindex_instance_parent_1".into(),
            "instance_parent".into(),
            None,
        ));
    }
    expected.sort_by(|left, right| left.1.cmp(&right.1));
    if actual != expected {
        return Err(SupplyError::CorruptStore);
    }
    let mut check = db.prepare("PRAGMA quick_check(1)").map_err(unavailable)?;
    let mut rows = check.query([]).map_err(unavailable)?;
    let first = rows
        .next()
        .map_err(unavailable)?
        .ok_or(SupplyError::CorruptStore)?;
    if first.get::<_, String>(0).map_err(unavailable)? != "ok"
        || rows.next().map_err(unavailable)?.is_some()
    {
        return Err(SupplyError::CorruptStore);
    }
    let mut foreign = db
        .prepare("PRAGMA foreign_key_check")
        .map_err(unavailable)?;
    if foreign
        .query([])
        .map_err(unavailable)?
        .next()
        .map_err(unavailable)?
        .is_some()
    {
        return Err(SupplyError::CorruptStore);
    }
    Ok(())
}

fn normalize(sql: &str) -> String {
    // 已冻结 SQL 不含字符串字面量；只兼容旧 DDL 的缩进，不接受改写后的约束或对象。
    sql.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

const PARENT_TABLE: &str = "CREATE TABLE instance_parent (
    child_id TEXT PRIMARY KEY REFERENCES agent_instance(id),
    parent_id TEXT NOT NULL REFERENCES agent_instance(id),
    CHECK (child_id != parent_id))";
const PARENT_INDEX: &str = "CREATE INDEX instance_parent_by_parent ON instance_parent(parent_id)";

/// 仅由显式管理命令调用；先验证完整旧格式，再在调用方事务内迁移。
pub(super) fn upgrade_children(db: &Connection) -> Result<(), SupplyError> {
    validate(db)?;
    if version(db)? != 0 {
        return Err(SupplyError::CorruptStore);
    }
    db.execute_batch(PARENT_TABLE).map_err(unavailable)?;
    db.execute_batch(PARENT_INDEX).map_err(unavailable)?;
    db.pragma_update(None, "user_version", 1)
        .map_err(unavailable)?;
    validate(db)
}
