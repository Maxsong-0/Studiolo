//! 格式 1：精确接纳既定表与索引，不隐式迁移，不把缺表当成初始化请求。

use rusqlite::{Connection, params};

use super::{PolicyError, unavailable};

/// SQLite `application_id`（ASCII "SPOL"）。
const APPLICATION_ID: i64 = 0x5350_4f4c;
const USER_VERSION: i64 = 1;

const TABLES: &[(&str, &str)] = &[
    (
        "policy_version",
        "CREATE TABLE policy_version (
        version INTEGER PRIMARY KEY,
        kind TEXT NOT NULL CHECK (kind IN ('rules','skills','capability_set')),
        subject TEXT NOT NULL CHECK (length(subject) BETWEEN 1 AND 128),
        body TEXT NOT NULL,
        body_sha256 TEXT NOT NULL CHECK (length(body_sha256) = 64),
        retired INTEGER NOT NULL CHECK (retired IN (0, 1)),
        effective_hash TEXT NOT NULL CHECK (length(effective_hash) = 64),
        previous_effective_hash TEXT NOT NULL CHECK (length(previous_effective_hash) = 64),
        activated_at TEXT NOT NULL,
        activated_by TEXT NOT NULL,
        desired_id TEXT,
        rollback_of INTEGER,
        expansions TEXT NOT NULL)",
    ),
    (
        "policy_current",
        "CREATE TABLE policy_current (
        kind TEXT NOT NULL,
        subject TEXT NOT NULL,
        version INTEGER NOT NULL REFERENCES policy_version(version),
        PRIMARY KEY (kind, subject))",
    ),
    (
        "policy_desired",
        "CREATE TABLE policy_desired (
        id TEXT PRIMARY KEY,
        kind TEXT NOT NULL CHECK (kind IN ('rules','skills','capability_set')),
        subject TEXT NOT NULL CHECK (length(subject) BETWEEN 1 AND 128),
        body TEXT,
        body_sha256 TEXT NOT NULL,
        submitter_kind TEXT NOT NULL CHECK (submitter_kind IN ('owner','agent')),
        agent_type TEXT,
        agent_instance TEXT,
        submitted_at TEXT NOT NULL,
        status TEXT NOT NULL CHECK (status IN ('pending','validated','rejected','activated','superseded','withdrawn')),
        report TEXT NOT NULL,
        validated_at TEXT,
        activated_version INTEGER)",
    ),
    (
        "policy_revocation",
        "CREATE TABLE policy_revocation (
        seq INTEGER PRIMARY KEY,
        level TEXT NOT NULL CHECK (level IN ('instance','type','global')),
        target TEXT NOT NULL,
        revoked_at TEXT NOT NULL,
        revoked_by TEXT NOT NULL,
        reason TEXT NOT NULL,
        released_at TEXT,
        released_by TEXT)",
    ),
    (
        "policy_meta",
        "CREATE TABLE policy_meta (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL)",
    ),
];

const INDEXES: &[(&str, &str, &str)] = &[
    (
        "version_by_subject",
        "policy_version",
        "CREATE INDEX version_by_subject ON policy_version(kind, subject, version)",
    ),
    (
        "desired_by_status",
        "policy_desired",
        "CREATE INDEX desired_by_status ON policy_desired(status, submitted_at)",
    ),
    (
        "desired_by_instance",
        "policy_desired",
        "CREATE INDEX desired_by_instance ON policy_desired(agent_instance, status)",
    ),
    (
        "revocation_active",
        "policy_revocation",
        "CREATE UNIQUE INDEX revocation_active ON policy_revocation(level, target) WHERE released_at IS NULL",
    ),
];

const AUTO_INDEXES: &[(&str, &str)] = &[
    ("sqlite_autoindex_policy_current_1", "policy_current"),
    ("sqlite_autoindex_policy_desired_1", "policy_desired"),
    ("sqlite_autoindex_policy_meta_1", "policy_meta"),
];

type SchemaObject = (String, String, String, Option<String>);

pub(super) fn initialize(db: &Connection, created: bool) -> Result<(), PolicyError> {
    if created {
        let objects: i64 = db
            .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
            .map_err(unavailable)?;
        if objects != 0 {
            return Err(PolicyError::CorruptStore);
        }
        db.pragma_update(None, "application_id", APPLICATION_ID)
            .map_err(unavailable)?;
        db.pragma_update(None, "user_version", USER_VERSION)
            .map_err(unavailable)?;
        for (_, sql) in TABLES {
            db.execute_batch(sql).map_err(unavailable)?;
        }
        for (_, _, sql) in INDEXES {
            db.execute_batch(sql).map_err(unavailable)?;
        }
        db.execute(
            "INSERT INTO policy_meta (key, value) VALUES ('effective_hash', ?1)",
            params![crate::sql::empty_effective_hash()],
        )
        .map_err(unavailable)?;
    }
    validate(db)
}

fn header(db: &Connection) -> Result<(), PolicyError> {
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(unavailable)?;
    let application: i64 = db
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .map_err(unavailable)?;
    if version != USER_VERSION || application != APPLICATION_ID {
        return Err(PolicyError::CorruptStore);
    }
    Ok(())
}

fn validate(db: &Connection) -> Result<(), PolicyError> {
    header(db)?;
    let count: i64 = db
        .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
        .map_err(unavailable)?;
    if count != (TABLES.len() + INDEXES.len() + AUTO_INDEXES.len()) as i64 {
        return Err(PolicyError::CorruptStore);
    }
    let invalid: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_schema
         WHERE typeof(type) != 'text' OR octet_length(type) NOT BETWEEN 1 AND 5
            OR typeof(name) != 'text' OR octet_length(name) NOT BETWEEN 1 AND 64
            OR typeof(tbl_name) != 'text' OR octet_length(tbl_name) NOT BETWEEN 1 AND 64
            OR (sql IS NOT NULL AND (typeof(sql) != 'text'
                OR octet_length(sql) NOT BETWEEN 1 AND ?1))",
            params![4096],
            |r| r.get(0),
        )
        .map_err(unavailable)?;
    if invalid != 0 {
        return Err(PolicyError::CorruptStore);
    }
    let mut statement = db
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY name")
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
    for (name, table, sql) in INDEXES {
        expected.push((
            "index".into(),
            (*name).into(),
            (*table).into(),
            Some(normalize(sql)),
        ));
    }
    for (name, table) in AUTO_INDEXES {
        expected.push(("index".into(), (*name).into(), (*table).into(), None));
    }
    expected.sort_by(|left, right| left.1.cmp(&right.1));
    if actual != expected {
        return Err(PolicyError::CorruptStore);
    }
    let mut check = db.prepare("PRAGMA quick_check(1)").map_err(unavailable)?;
    let mut rows = check.query([]).map_err(unavailable)?;
    let first = rows
        .next()
        .map_err(unavailable)?
        .ok_or(PolicyError::CorruptStore)?;
    if first.get::<_, String>(0).map_err(unavailable)? != "ok"
        || rows.next().map_err(unavailable)?.is_some()
    {
        return Err(PolicyError::CorruptStore);
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
        return Err(PolicyError::CorruptStore);
    }
    let meta: Option<String> = db
        .query_row(
            "SELECT value FROM policy_meta WHERE key = 'effective_hash'",
            [],
            |r| r.get(0),
        )
        .ok();
    if meta.is_none_or(|value| value.len() != 64) {
        return Err(PolicyError::CorruptStore);
    }
    Ok(())
}

fn normalize(sql: &str) -> String {
    sql.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}
