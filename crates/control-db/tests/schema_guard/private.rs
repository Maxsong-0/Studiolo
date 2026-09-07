//! 仅作为 schema_guard 私有子模块编译，验证预检后的版本推进。

use std::fs;

use studiolo_core::VaultUuid;
use tempfile::TempDir;

use super::*;
use crate::ControlDb;

#[test]
fn migration_rechecks_a_schema_that_advanced_after_preflight() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("advanced.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (
                 version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);
             INSERT INTO schema_migrations VALUES (2, 0);",
        )
        .unwrap();
    require_supported_version(&connection).unwrap();
    {
        let other = Connection::open(&path).unwrap();
        other
            .execute("INSERT INTO schema_migrations VALUES (99, 1)", [])
            .unwrap();
    }
    let before = fs::read(&path).unwrap();
    let mut db = ControlDb {
        connection,
        vault_uuid: VaultUuid::new(),
    };

    assert!(matches!(
        db.migrate(),
        Err(DbError::UnsupportedSchema {
            found: 99,
            supported: 2,
        })
    ));
    drop(db);
    assert!(fs::read(&path).unwrap() == before, "迁移复核不得改写新版库");
}
