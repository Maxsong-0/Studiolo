//! 真实 SQLite 开库契约：旧格式 0 不迁移，既有损坏状态两个入口均不能隐式修补。
#![cfg(unix)]

#[path = "schema_open/fixture.rs"]
mod fixture;

use std::{fs, os::unix::fs::PermissionsExt};

use fixture::{Fixture, LEGACY_SQL};
use sha2::{Digest as _, Sha256};
use studiolo_supply::{AgentStore, SupplyError};

#[test]
fn only_exclusively_created_file_is_initialized_and_format_zero_is_preserved() {
    let f = Fixture::empty();
    assert!(matches!(
        AgentStore::open_existing(&f.root),
        Err(SupplyError::MissingStore)
    ));
    assert!(!f.database().exists());
    let store = AgentStore::open(&f.root).unwrap();
    let token = store
        .issue_type("claude", "2026-09-13T00:00:00Z")
        .unwrap()
        .token;
    drop(store);
    assert_eq!(f.value::<i64>("PRAGMA user_version"), 0);
    assert_eq!(f.value::<i64>("PRAGMA application_id"), 0);
    let store = AgentStore::open_existing(&f.root).unwrap();
    assert_eq!(
        store.authenticate(&token).unwrap().as_deref(),
        Some("claude")
    );
}

#[test]
fn legacy_zero_schema_keeps_credentials_revocation_and_instances_in_both_open_modes() {
    for wal in [false, true] {
        let f = Fixture::legacy();
        let token = format!("sag1_{}", "b".repeat(64));
        let digest: String = Sha256::digest(token.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let db = rusqlite::Connection::open(f.database()).unwrap();
        db.execute(
            "INSERT INTO agent_type VALUES('claude',?1,'2026-09-13T00:00:00Z',NULL)",
            [&digest],
        )
        .unwrap();
        db.execute("INSERT INTO agent_type VALUES('codex',?1,'2026-09-13T00:00:00Z','2026-09-13T00:00:01Z')",
            ["c".repeat(64)]).unwrap();
        db.execute(
            "INSERT INTO agent_instance VALUES('legacy-instance','claude',?1,'before','before')",
            ["d".repeat(64)],
        )
        .unwrap();
        if wal {
            db.pragma_update(None, "journal_mode", "WAL").unwrap();
        }
        db.close().unwrap();
        for open in [AgentStore::open_existing, AgentStore::open] {
            let store = open(&f.root).unwrap();
            assert_eq!(
                store.authenticate(&token).unwrap().as_deref(),
                Some("claude")
            );
            assert!(!store.type_is_active("codex").unwrap());
            assert_eq!(store.types().unwrap().len(), 2);
            drop(store);
            assert_eq!(f.value::<i64>("PRAGMA user_version"), 0);
            assert_eq!(f.value::<i64>("PRAGMA application_id"), 0);
            assert_eq!(f.value::<i64>("SELECT count(*) FROM agent_instance"), 1);
        }
    }
}

#[test]
fn missing_table_is_not_recreated_through_either_open_entry() {
    for table in ["agent_type", "agent_instance", "reads", "outage"] {
        let f = Fixture::legacy();
        f.modify(&format!("DROP TABLE {table}"));
        f.rejected_unchanged();
    }
}

#[test]
fn missing_or_redefined_index_is_not_repaired() {
    for index in [
        "reads_by_type",
        "reads_by_instance",
        "reads_by_path",
        "reads_by_time",
    ] {
        let f = Fixture::legacy();
        f.modify(&format!("DROP INDEX {index}"));
        f.rejected_unchanged();
    }
    let f = Fixture::legacy();
    f.modify("DROP INDEX reads_by_type; CREATE INDEX reads_by_type ON reads(path)");
    f.rejected_unchanged();
}

#[test]
fn unexpected_table_index_view_or_trigger_is_rejected_before_wal_changes() {
    for extra in [
        "CREATE TABLE foreign_store (id INTEGER)",
        "CREATE INDEX extra_index ON agent_type(issued_at)",
        "CREATE VIEW extra_view AS SELECT name FROM agent_type",
        "CREATE TRIGGER extra_trigger AFTER INSERT ON agent_type BEGIN DELETE FROM agent_type; END",
    ] {
        let f = Fixture::legacy();
        f.modify(extra);
        f.rejected_unchanged();
    }
}

#[test]
fn changed_constraints_or_column_shapes_are_rejected_with_same_object_count() {
    for (before, after) in [
        (
            "token_sha256 TEXT NOT NULL UNIQUE",
            "token_sha256 TEXT UNIQUE",
        ),
        (
            "length(name) BETWEEN 1 AND 32",
            "length(name) BETWEEN 1 AND 64",
        ),
        (
            "session_digest TEXT NOT NULL",
            "session_digest BLOB NOT NULL",
        ),
        (
            "denied_count INTEGER NOT NULL DEFAULT 0",
            "denied_count INTEGER NOT NULL DEFAULT 1",
        ),
    ] {
        let f = Fixture::with_sql(&LEGACY_SQL.replace(before, after));
        f.rejected_unchanged();
    }
}

#[test]
fn unknown_schema_version_or_application_id_is_not_silently_adopted() {
    for update in [
        "PRAGMA user_version=1",
        "PRAGMA user_version=-1",
        "PRAGMA application_id=1398030677",
    ] {
        let f = Fixture::legacy();
        f.modify(update);
        f.rejected_unchanged();
    }
}

#[test]
fn sqlite_file_without_agent_schema_is_not_an_initialization_request() {
    for sql in ["CREATE TABLE other (id INTEGER)", "VACUUM"] {
        let f = Fixture::with_sql(sql);
        assert!(fs::metadata(f.database()).unwrap().len() > 0);
        f.rejected_unchanged();
    }
}

#[test]
fn orphan_instance_and_broken_check_constraint_are_rejected() {
    let f = Fixture::legacy();
    f.modify(
        "PRAGMA foreign_keys=OFF;
        INSERT INTO agent_instance VALUES('orphan','missing','digest','before','before')",
    );
    f.rejected_unchanged();
    let f = Fixture::legacy();
    f.modify(
        "PRAGMA ignore_check_constraints=ON;
        INSERT INTO agent_type VALUES('','bad','before',NULL)",
    );
    f.rejected_unchanged();
}

#[test]
fn oversized_schema_text_is_rejected_before_wal_changes() {
    let expanded = LEGACY_SQL.replace(
        "revoked_at TEXT",
        &format!("{}revoked_at TEXT", " ".repeat(4096)),
    );
    let f = Fixture::with_sql(&expanded);
    f.rejected_unchanged();
}

#[test]
fn existing_empty_or_non_sqlite_file_is_never_replaced() {
    for content in [b"".as_slice(), b"not a sqlite database".as_slice()] {
        let f = Fixture::empty();
        fs::write(f.database(), content).unwrap();
        fs::set_permissions(f.database(), fs::Permissions::from_mode(0o600)).unwrap();
        for open in [AgentStore::open_existing, AgentStore::open] {
            assert!(open(&f.root).is_err());
            assert_eq!(fs::read(f.database()).unwrap(), content);
        }
    }
}
