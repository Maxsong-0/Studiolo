//! 独立保留旧发布 DDL，验证真实格式 0 的兼容性，不复用生产 schema 常量。

use std::{
    fs,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use rusqlite::Connection;
use studiolo_supply::{AgentStore, SupplyError};
use tempfile::TempDir;

pub const LEGACY_SQL: &str = include_str!("legacy.sql");

pub struct Fixture {
    _temp: TempDir,
    pub root: PathBuf,
}

impl Fixture {
    pub fn empty() -> Self {
        let temp = TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let root = temp.path().join("agent");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        Self { _temp: temp, root }
    }

    pub fn legacy() -> Self {
        Self::with_sql(LEGACY_SQL)
    }

    pub fn with_sql(sql: &str) -> Self {
        let fixture = Self::empty();
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(fixture.database())
            .unwrap();
        fixture.modify(sql);
        fixture
    }

    pub fn database(&self) -> PathBuf {
        self.root.join("agent.sqlite3")
    }

    pub fn modify(&self, sql: &str) {
        let db = Connection::open(self.database()).unwrap();
        db.execute_batch(sql).unwrap();
        db.close().unwrap();
    }

    pub fn value<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        let db = Connection::open(self.database()).unwrap();
        db.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    pub fn rejected_unchanged(&self) {
        let before = fs::read(self.database()).unwrap();
        for open in [AgentStore::open_existing, AgentStore::open] {
            assert!(matches!(open(&self.root), Err(SupplyError::CorruptStore)));
            assert_eq!(fs::read(self.database()).unwrap(), before);
            // 所有负测使用 DELETE journal；拒绝不得先将其切到 WAL。
            assert_eq!(self.value::<String>("PRAGMA journal_mode"), "delete");
            for suffix in ["-wal", "-shm", "-journal"] {
                assert!(!Path::new(&format!("{}{suffix}", self.database().display())).exists());
            }
        }
    }
}
