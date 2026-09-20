//! 新版库拒绝的副作用契约：夹具由原始 SQLite 创建，并在检查前干净关闭。
//! 不把静止夹具的字节保持外推为热 journal 或外部并发写者下的物理零写承诺。

use std::{fs, path::Path};

use rusqlite::{Connection, OpenFlags};
use studiolo_control_db::{ControlDb, DbError};
use studiolo_core::VaultUuid;
use tempfile::TempDir;

#[derive(Debug, PartialEq, Eq)]
struct SchemaEntry {
    kind: String,
    name: String,
    table: String,
    root_page: i64,
    sql: Option<String>,
}

struct DatabaseSnapshot {
    journal_mode: String,
    schema: Vec<SchemaEntry>,
    migrations: Vec<(i64, i64)>,
    payload: Vec<(i64, Vec<u8>, String)>,
    bytes: Vec<u8>,
}

fn create_newer_database(path: &Path, journal_mode: &str) {
    let raw = Connection::open(path).expect("创建原始 SQLite 夹具");
    let pragma = match journal_mode {
        "delete" => "PRAGMA journal_mode=DELETE",
        "wal" => "PRAGMA journal_mode=WAL",
        _ => panic!("未知夹具 journal mode"),
    };
    let actual: String = raw.query_row(pragma, [], |row| row.get(0)).unwrap();
    assert_eq!(actual, journal_mode);
    raw.execute_batch(
        "CREATE TABLE schema_migrations (
             version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);
         INSERT INTO schema_migrations VALUES (2, 123), (99, 456);
         CREATE TABLE future_payload (
             id INTEGER PRIMARY KEY, body BLOB NOT NULL, label TEXT NOT NULL);
         CREATE UNIQUE INDEX future_label ON future_payload(label);
         INSERT INTO future_payload VALUES
             (1, X'00017FFF', '未来版本哨兵一'),
             (2, X'FF804100', '未来版本哨兵二');",
    )
    .expect("写入新版 schema 与哨兵");
    if journal_mode == "wal" {
        let checkpoint: (i64, i64, i64) = raw
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        assert_eq!(checkpoint, (0, 0, 0));
    }
    raw.close().expect("干净关闭原始夹具");
}

fn snapshot(path: &Path) -> DatabaseSnapshot {
    let (journal_mode, schema, migrations, payload) = {
        let raw = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("只读检查夹具");
        let journal_mode = raw
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        let schema = raw
            .prepare("SELECT type,name,tbl_name,rootpage,sql FROM sqlite_schema ORDER BY type,name")
            .unwrap()
            .query_map([], |row| {
                Ok(SchemaEntry {
                    kind: row.get(0)?,
                    name: row.get(1)?,
                    table: row.get(2)?,
                    root_page: row.get(3)?,
                    sql: row.get(4)?,
                })
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let migrations = raw
            .prepare("SELECT version,applied_at_ms FROM schema_migrations ORDER BY version")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let payload = raw
            .prepare("SELECT id,body,label FROM future_payload ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (journal_mode, schema, migrations, payload)
    };
    DatabaseSnapshot {
        journal_mode,
        schema,
        migrations,
        payload,
        bytes: fs::read(path).expect("读取关闭后的数据库字节"),
    }
}

fn assert_newer_database_unchanged(journal_mode: &str) {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("future.sqlite3");
    create_newer_database(&path, journal_mode);
    let before = snapshot(&path);
    assert_eq!(before.journal_mode, journal_mode);

    assert!(matches!(
        ControlDb::open(&path, VaultUuid::new()),
        Err(DbError::UnsupportedSchema {
            found: 99,
            supported: 3,
        })
    ));

    let after = snapshot(&path);
    assert_eq!(after.journal_mode, before.journal_mode);
    assert_eq!(after.schema, before.schema);
    assert_eq!(after.migrations, before.migrations);
    assert_eq!(after.payload, before.payload);
    assert!(
        after.bytes == before.bytes,
        "新版库拒绝后数据库字节发生变化"
    );
}

#[test]
fn clean_newer_delete_database_is_rejected_without_persistent_changes() {
    assert_newer_database_unchanged("delete");
}

#[test]
fn clean_newer_wal_database_is_rejected_without_persistent_changes() {
    assert_newer_database_unchanged("wal");
}

#[test]
fn an_empty_existing_migration_ledger_initializes_and_keeps_runtime_pragmas() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("empty-ledger.sqlite3");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "CREATE TABLE schema_migrations (
             version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);",
    )
    .unwrap();
    raw.close().unwrap();
    let vault = VaultUuid::new();

    let db = ControlDb::open(&path, vault).unwrap();
    assert_eq!(db.schema_version().unwrap(), 3);
    assert_eq!(db.bound_vault_uuid().unwrap(), vault);
    assert_eq!(db.pragma("journal_mode").unwrap(), "wal");
    assert_eq!(db.pragma("synchronous").unwrap(), "2");
    assert_eq!(db.pragma("foreign_keys").unwrap(), "1");
    drop(db);
    assert_eq!(
        ControlDb::open(&path, vault)
            .unwrap()
            .schema_version()
            .unwrap(),
        3
    );
}

#[test]
fn in_memory_initialization_keeps_runtime_pragmas() {
    let db = ControlDb::open_in_memory(VaultUuid::new()).unwrap();
    assert_eq!(db.schema_version().unwrap(), 3);
    assert_eq!(db.pragma("journal_mode").unwrap(), "memory");
    assert_eq!(db.pragma("synchronous").unwrap(), "2");
    assert_eq!(db.pragma("foreign_keys").unwrap(), "1");
}

#[test]
fn a_case_variant_ledger_name_cannot_bypass_the_newer_version_check() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("case-variant.sqlite3");
    create_newer_database(&path, "delete");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "ALTER TABLE schema_migrations RENAME TO migration_ledger_renaming;
         ALTER TABLE migration_ledger_renaming RENAME TO Schema_Migrations;",
    )
    .unwrap();
    raw.close().unwrap();
    let before = fs::read(&path).unwrap();

    assert!(matches!(
        ControlDb::open(&path, VaultUuid::new()),
        Err(DbError::UnsupportedSchema {
            found: 99,
            supported: 3,
        })
    ));
    assert!(fs::read(&path).unwrap() == before);
}

#[test]
fn a_view_cannot_substitute_for_the_migration_ledger() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("view-ledger.sqlite3");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch("CREATE VIEW schema_migrations AS SELECT 99 AS version")
        .unwrap();
    raw.close().unwrap();
    let before = fs::read(&path).unwrap();

    assert!(matches!(
        ControlDb::open(&path, VaultUuid::new()),
        Err(DbError::Corrupt(_))
    ));
    assert!(fs::read(&path).unwrap() == before);
}
