//! 审计私有目录、链接、权限与特殊节点的负向契约。

#![cfg(unix)]

mod support;

use serde_json::json;
use std::{
    fs,
    os::unix::{
        fs::{PermissionsExt, symlink},
        net::UnixListener,
    },
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, SanitizedPayload,
    SqliteAuditLedger,
};

fn event() -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse("storage-event").unwrap(),
        AuditPhase::Intent,
        "test.action",
        SanitizedPayload::try_from(json!({})).unwrap(),
    )
}

#[test]
fn unsafe_directory_and_database_targets_are_rejected_without_following_links() {
    let parent = support::private_dir();
    let root = parent.path().canonicalize().unwrap();
    assert!(SqliteAuditLedger::open(&root.join("missing")).is_err());
    assert!(!root.join("missing").exists());
    assert!(SqliteAuditLedger::open(std::path::Path::new("relative")).is_err());
    let private = root.join("private");
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&private, root.join("alias")).unwrap();
    assert!(SqliteAuditLedger::open(&root.join("alias")).is_err());
    assert!(SqliteAuditLedger::open(&root.join("alias/child")).is_err());
    fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(SqliteAuditLedger::open(&private).is_err());
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    let outside = root.join("outside");
    fs::write(&outside, b"do not modify").unwrap();
    symlink(&outside, private.join("audit.sqlite3")).unwrap();
    assert!(SqliteAuditLedger::open(&private).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"do not modify");
}

#[test]
fn unsafe_sidecars_and_special_nodes_are_rejected_before_sqlite_opens() {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let dir = support::private_dir();
        let root = dir.path().canonicalize().unwrap();
        let target = root.join(format!("audit.sqlite3{suffix}"));
        let _socket = UnixListener::bind(&target).unwrap();
        assert!(SqliteAuditLedger::open(&root).is_err());
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        let dir = support::private_dir();
        let root = dir.path().canonicalize().unwrap();
        let target = root.join(format!("audit.sqlite3{suffix}"));
        symlink("absent", target).unwrap();
        assert!(SqliteAuditLedger::open(&root).is_err());
        assert!(!root.join("absent").exists());
    }
}

#[test]
fn hardlinks_permissions_and_live_identity_replacement_fail_closed() {
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    let record = ledger.append(event()).unwrap();
    let db = root.join("audit.sqlite3");
    fs::hard_link(&db, root.join("alias")).unwrap();
    assert_eq!(
        ledger.append(event()).unwrap_err(),
        AuditError::UnsafeStorage
    );
    fs::remove_file(root.join("alias")).unwrap();
    fs::set_permissions(&db, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        ledger.query(&AuditFilter::default()).unwrap_err(),
        AuditError::UnsafeStorage
    );
    fs::set_permissions(&db, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap(), vec![record]);
    fs::rename(&db, root.join("previous.sqlite3")).unwrap();
    fs::write(&db, b"replacement").unwrap();
    fs::set_permissions(&db, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        ledger.append(event()).unwrap_err(),
        AuditError::UnsafeStorage
    );
}

#[test]
fn oversized_record_does_not_advance_the_ledger() {
    let dir = support::private_dir();
    let ledger = SqliteAuditLedger::open(&dir.path().canonicalize().unwrap()).unwrap();
    let event = AuditEvent::new(
        AuditEventId::parse("oversized").unwrap(),
        AuditPhase::Intent,
        "test.action",
        SanitizedPayload::try_from(json!({"summary": "x".repeat(1024 * 1024)})).unwrap(),
    );
    assert_eq!(
        ledger.append(event).unwrap_err(),
        AuditError::RecordTooLarge
    );
    assert!(ledger.query(&AuditFilter::default()).unwrap().is_empty());
}
