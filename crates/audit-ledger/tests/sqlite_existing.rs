//! 只打开既有审计库的公开契约；全部目标均为测试拥有的临时私有目录。

#![cfg(unix)]

mod support;

use serde_json::json;
use std::{
    fs,
    os::unix::{
        fs::{PermissionsExt, symlink},
        net::UnixListener,
    },
    path::Path,
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, SanitizedPayload,
    SqliteAuditLedger,
};

fn event(phase: AuditPhase) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse("existing-only").unwrap(),
        phase,
        "test.action",
        SanitizedPayload::try_from(json!({"decision":"allow"})).unwrap(),
    )
}

fn private_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn missing_database_never_creates_a_main_file_or_sidecar() {
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    for _ in 0..2 {
        assert_eq!(
            SqliteAuditLedger::open_existing(&root).err().unwrap(),
            AuditError::MissingStore
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }
    // 原初始化入口仍能明确建立新库；失败的只打开入口没有留下空文件。
    drop(SqliteAuditLedger::open(&root).unwrap());
    assert!(SqliteAuditLedger::open_existing(&root).is_ok());
}

#[test]
fn existing_ledger_retains_exact_receipts_and_remains_appendable() {
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    let initial = SqliteAuditLedger::open(&root).unwrap();
    let original = initial.append(event(AuditPhase::Intent)).unwrap();
    drop(initial);
    let reopened = SqliteAuditLedger::open_existing(&root).unwrap();
    assert_eq!(
        reopened.query(&AuditFilter::default()).unwrap(),
        vec![original.clone()]
    );
    assert_eq!(
        reopened.append(event(AuditPhase::Intent)).unwrap(),
        original
    );
    let outcome = reopened.append(event(AuditPhase::Outcome)).unwrap();
    assert_eq!(outcome.offset(), 2);
    assert_eq!(outcome.previous_hash(), original.record_hash());
    drop(reopened);
    let reopened = SqliteAuditLedger::open_existing(&root).unwrap();
    assert_eq!(
        reopened.query(&AuditFilter::default()).unwrap(),
        vec![original, outcome]
    );
}

#[test]
fn empty_and_non_sqlite_files_are_never_initialized_or_repaired() {
    for bytes in [b"".as_slice(), b"not an audit database".as_slice()] {
        let dir = support::private_dir();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("audit.sqlite3");
        private_file(&file, bytes);
        assert!(SqliteAuditLedger::open_existing(&root).is_err());
        assert_eq!(fs::read(&file).unwrap(), bytes);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn unknown_schema_and_corrupt_chain_remain_rejected() {
    for change in [
        "PRAGMA user_version=99",
        "UPDATE audit_meta SET last_hash='broken'",
    ] {
        let dir = support::private_dir();
        let root = dir.path().canonicalize().unwrap();
        let initial = SqliteAuditLedger::open(&root).unwrap();
        initial.append(event(AuditPhase::Intent)).unwrap();
        drop(initial);
        let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
        connection.execute_batch(change).unwrap();
        drop(connection);
        assert!(SqliteAuditLedger::open_existing(&root).is_err());
        assert!(SqliteAuditLedger::open(&root).is_err());
    }
}

#[test]
fn symlinked_main_or_sidecar_does_not_touch_the_target() {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let dir = support::private_dir();
        let root = dir.path().canonicalize().unwrap();
        let outside = root.join("outside");
        private_file(&outside, b"unchanged");
        symlink(&outside, root.join(format!("audit.sqlite3{suffix}"))).unwrap();
        assert_eq!(
            SqliteAuditLedger::open_existing(&root).err().unwrap(),
            AuditError::UnsafeStorage
        );
        assert_eq!(fs::read(&outside).unwrap(), b"unchanged");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    }
}

#[test]
fn permissions_and_hardlinks_are_rejected_without_silent_repairs() {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let dir = support::private_dir();
        let root = dir.path().canonicalize().unwrap();
        drop(SqliteAuditLedger::open(&root).unwrap());
        let target = root.join(format!("audit.sqlite3{suffix}"));
        if !suffix.is_empty() {
            private_file(&target, b"invalid sidecar");
        }
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            SqliteAuditLedger::open_existing(&root).err().unwrap(),
            AuditError::UnsafeStorage
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    drop(SqliteAuditLedger::open(&root).unwrap());
    fs::hard_link(root.join("audit.sqlite3"), root.join("alias")).unwrap();
    assert_eq!(
        SqliteAuditLedger::open_existing(&root).err().unwrap(),
        AuditError::UnsafeStorage
    );
}

#[test]
fn invalid_directory_inputs_never_create_or_follow_a_directory() {
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    assert!(SqliteAuditLedger::open_existing(&root.join("missing")).is_err());
    assert!(!root.join("missing").exists());
    assert!(SqliteAuditLedger::open_existing(Path::new("relative")).is_err());
    symlink(&root, root.join("alias")).unwrap();
    assert!(SqliteAuditLedger::open_existing(&root.join("alias")).is_err());
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(SqliteAuditLedger::open_existing(&root).is_err());
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(!root.join("audit.sqlite3").exists());
}

#[test]
fn special_nodes_are_rejected_before_opening_sqlite() {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let dir = support::private_dir();
        let root = dir.path().canonicalize().unwrap();
        let _socket = UnixListener::bind(root.join(format!("audit.sqlite3{suffix}"))).unwrap();
        assert_eq!(
            SqliteAuditLedger::open_existing(&root).err().unwrap(),
            AuditError::UnsafeStorage
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }
}
