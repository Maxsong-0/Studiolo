//! 不创建打开的公开契约；只使用私有临时目录和真实 SQLite。

use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::atomic::Ordering,
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

fn existing(h: &Harness) -> Result<SessionManager, SessionError> {
    SessionManager::open_existing(
        &h.root,
        config(10, 100),
        h.audit.clone(),
        h.clock.clone(),
        h.random.clone(),
    )
}

#[test]
fn absent_fixed_database_is_rejected_without_files_or_randomness() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    let h = Harness::new(10, 100);
    let random_before = h.random.next.load(Ordering::SeqCst);
    for _ in 0..2 {
        let result = SessionManager::open_existing(
            &root,
            config(10, 100),
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone(),
        );
        assert_eq!(result.err().unwrap(), SessionError::MissingStore);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }
    assert_eq!(h.random.next.load(Ordering::SeqCst), random_before);
    assert!(h.audit.query(&AuditFilter::default()).unwrap().is_empty());
}

#[test]
fn missing_directory_and_existing_empty_database_are_never_initialized() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    let h = Harness::new(10, 100);
    let absent = root.join("absent");
    assert!(
        SessionManager::open_existing(
            &absent,
            config(10, 100),
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone()
        )
        .is_err()
    );
    assert!(!absent.exists());
    let database = root.join("auth.sqlite3");
    fs::write(&database, []).unwrap();
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    let result = SessionManager::open_existing(
        &root,
        config(10, 100),
        h.audit.clone(),
        h.clock.clone(),
        h.random.clone(),
    );
    assert_eq!(result.err().unwrap(), SessionError::CorruptStore);
    assert_eq!(fs::metadata(database).unwrap().len(), 0);
    assert_eq!(fs::read_dir(root).unwrap().count(), 1);
}

#[test]
fn existing_v2_v3_v4_open_keeps_format_credentials_and_audit_without_randomness() {
    for version in 2..=4 {
        let h = Harness::new(10, 100);
        let issued = h.manager.issue(&identity()).unwrap();
        if version >= 3 {
            SessionManager::migrate_v2(
                &h.root,
                config(10, 100),
                h.audit.clone(),
                h.clock.clone(),
                h.random.clone(),
            )
            .unwrap();
        }
        if version == 4 {
            SessionManager::migrate_v3(
                &h.root,
                config(10, 100),
                h.audit.clone(),
                h.clock.clone(),
                h.random.clone(),
            )
            .unwrap();
        }
        let random_before = h.random.next.load(Ordering::SeqCst);
        let audit_before = h.audit.query(&AuditFilter::default()).unwrap();
        h.random.fail.store(true, Ordering::SeqCst);
        h.audit.fail.store(true, Ordering::SeqCst);
        let reopened = existing(&h).unwrap();
        assert_eq!(scalar(&reopened, "PRAGMA user_version"), version);
        assert_eq!(
            reopened.check(&identity(), &issued.token).unwrap().id,
            issued.session.id
        );
        assert_eq!(h.random.next.load(Ordering::SeqCst), random_before);
        assert_eq!(
            h.audit.query(&AuditFilter::default()).unwrap(),
            audit_before
        );
    }
}

#[test]
fn existing_v1_requires_migration_without_altering_legacy_evidence() {
    let old = super::migration_fixture::LegacyFixture::new();
    let before = old.snapshot();
    assert_eq!(
        existing(&old.h).err().unwrap(),
        SessionError::MigrationRequired
    );
    assert_eq!(old.snapshot(), before);
    assert_eq!(scalar(&old.h.manager, "PRAGMA user_version"), 1);
}

#[test]
fn existing_open_keeps_config_integrity_and_clock_checks() {
    let h = Harness::new(10, 100);
    let issued = h.manager.issue(&identity()).unwrap();
    let mismatch = SessionManager::open_existing(
        &h.root,
        config(11, 100),
        h.audit.clone(),
        h.clock.clone(),
        h.random.clone(),
    );
    assert_eq!(mismatch.err().unwrap(), SessionError::Configuration);
    h.clock.set(1_010_000);
    let reopened = existing(&h).unwrap();
    assert_eq!(
        reopened.check(&identity(), &issued.token).unwrap_err(),
        SessionError::Rejected
    );
    h.clock.set(1_000_000);
    assert_eq!(existing(&h).err().unwrap(), SessionError::ClockRollback);
    assert_eq!(
        scalar(&h.manager, "SELECT clock_floor FROM auth_meta"),
        1_010_000
    );
    h.clock.set(1_010_000);
    h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute_batch("PRAGMA user_version=99")
        .unwrap();
    assert_eq!(existing(&h).err().unwrap(), SessionError::UnsupportedSchema);
    assert_eq!(scalar(&h.manager, "PRAGMA user_version"), 99);
}

#[test]
fn existing_open_rejects_links_and_permissions_without_repairing_targets() {
    for hardlink in [false, true] {
        let dir = directory();
        let root = dir.path().canonicalize().unwrap();
        let h = Harness::new(10, 100);
        let target = root.join("preserved");
        fs::write(&target, b"not a database").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let database = root.join("auth.sqlite3");
        if hardlink {
            fs::hard_link(&target, &database).unwrap();
        } else {
            symlink(&target, &database).unwrap();
        }
        let result = SessionManager::open_existing(
            &root,
            config(10, 100),
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone(),
        );
        assert_eq!(result.err().unwrap(), SessionError::UnsafeStorage);
        assert_eq!(fs::read(&target).unwrap(), b"not a database");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    }
    let h = Harness::new(10, 100);
    let database = h.root.join("auth.sqlite3");
    fs::set_permissions(&database, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(existing(&h).err().unwrap(), SessionError::UnsafeStorage);
    assert_eq!(
        fs::metadata(&database).unwrap().permissions().mode() & 0o7777,
        0o644
    );
}
