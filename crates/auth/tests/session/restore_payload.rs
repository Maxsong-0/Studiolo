//! restore 单独严格闭集，普通 one/all 不接受零计数或恢复扩展字段。
use super::*;
use serde_json::{Value, json};

fn accepted(id: &str, value: Value) -> bool {
    let Ok(event) = events::event(id, AuditPhase::Outcome, "session.revoked", value) else {
        return false;
    };
    event_validation::payload(&event, "revoke", None, None, "test:").is_ok()
}

fn good() -> Value {
    json!({"scope":"restore","target":"r-1","revoked":0,"identity_version":2,
        "effective":"already_revoked","cleared_l3":0,"invalidated_recovery_code_count":0,"version":1})
}

#[test]
fn exact_payload_and_namespace_are_closed_without_relaxing_regular_revocation() {
    assert!(accepted("test:restore:r-1", good()));
    for (field, value) in [
        ("version", json!(2)),
        ("version", json!(1.0)),
        ("revoked", json!(-1)),
        ("revoked", json!(1025)),
        ("revoked", json!(0.0)),
        ("cleared_l3", json!(1025)),
        ("invalidated_recovery_code_count", json!(1)),
        ("identity_version", json!(1)),
        ("effective", json!("pending")),
        ("target", json!("r_1")),
        ("target", json!("x".repeat(56))),
        ("extra", json!(false)),
        ("scope", json!("all")),
    ] {
        let mut value_all = good();
        value_all[field] = value;
        assert!(!accepted("test:restore:r-1", value_all), "{field}");
    }
    let mut missing = good();
    missing.as_object_mut().unwrap().remove("version");
    assert!(!accepted("test:restore:r-1", missing));
    assert!(!accepted("wrong:restore:r-1", good()));
    assert!(!accepted("test:restore:other", good()));
    for (scope, target, id) in [
        ("one", "s", "test:revoke:s"),
        ("all", "all", "test:revoke-all:2"),
    ] {
        let mut value = json!({"scope":scope,"target":target,"revoked":1,"identity_version":2,"effective":"already_revoked"});
        assert!(accepted(id, value.clone()));
        value["revoked"] = json!(0);
        assert!(!accepted(id, value.clone()));
        value["revoked"] = json!(1);
        value["version"] = json!(1);
        assert!(!accepted(id, value));
    }
}

#[test]
fn duplicated_epoch_or_mismatched_pair_is_rejected_on_reopen() {
    for duplicate in [false, true] {
        let h = RecoveryHarness::new(3600, 7200);
        h.h.manager
            .invalidate_restored_credentials("original")
            .unwrap();
        if duplicate {
            h.h.manager
                .database
                .transaction(|db| {
                    let store = schema::store_id(db)?;
                    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
                        let mut value = good();
                        value["target"] = json!("duplicate");
                        let event = events::event(
                            &format!("{store}:restore:duplicate"),
                            phase,
                            "session.revoked",
                            value,
                        )?;
                        outbox::enqueue(db, &event, "revoke", None)?;
                    }
                    Ok(())
                })
                .unwrap();
        } else {
            execute(
                &h.h.manager,
                "DELETE FROM audit_outbox WHERE event_id LIKE '%:restore:%' AND phase='intent'",
            );
        }
        assert!(
            SessionManager::open(
                &h.h.root,
                config(3600, 7200),
                h.h.audit.clone(),
                h.h.clock.clone(),
                h.h.random.clone()
            )
            .is_err()
        );
    }
}

#[test]
fn restore_open_does_not_create_or_migrate_and_retains_original_storage_guards() {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    let h = Harness::new(10, 100);
    let open = |root: &std::path::Path, cfg: SessionConfig| {
        SessionManager::open_existing_for_restore(
            root,
            cfg,
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone(),
        )
    };
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    let absent = root.join("missing");
    assert!(open(&absent, config(10, 100)).is_err());
    assert!(!absent.exists());
    let random = h.random.next.load(Ordering::SeqCst);
    assert_eq!(
        open(&root, config(10, 100)).err().unwrap(),
        SessionError::MissingStore
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    assert_eq!(h.random.next.load(Ordering::SeqCst), random);
    assert_eq!(
        open(&h.root, config(10, 100)).err().unwrap(),
        SessionError::MigrationRequired
    );
    assert_eq!(scalar(&h.manager, "PRAGMA user_version"), 2);
    SessionManager::migrate_v2(
        &h.root,
        config(10, 100),
        h.audit.clone(),
        h.clock.clone(),
        h.random.clone(),
    )
    .unwrap();
    let before = whole(&h.manager);
    assert_eq!(
        open(&h.root, config(10, 100)).err().unwrap(),
        SessionError::MigrationRequired
    );
    assert_eq!(scalar(&h.manager, "PRAGMA user_version"), 3);
    assert_eq!(whole(&h.manager), before);
    let target = root.join("preserved");
    fs::write(&target, b"not a database").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&target, root.join("auth.sqlite3")).unwrap();
    assert_eq!(
        open(&root, config(10, 100)).err().unwrap(),
        SessionError::UnsafeStorage
    );
    assert_eq!(fs::read(&target).unwrap(), b"not a database");
}

#[test]
fn restore_open_keeps_v4_owner_corruption_and_permissions_checks() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let h = RecoveryHarness::new(3600, 7200);
    h.ready();
    let open = |cfg| {
        SessionManager::open_existing_for_restore(
            &h.h.root,
            cfg,
            h.h.audit.clone(),
            h.h.clock.clone(),
            h.h.random.clone(),
        )
    };
    let before = whole(&h.h.manager);
    assert_eq!(
        open(config(3601, 7200)).err().unwrap(),
        SessionError::Configuration
    );
    assert_eq!(whole(&h.h.manager), before);
    let database = h.h.root.join("auth.sqlite3");
    fs::set_permissions(&database, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        open(config(3600, 7200)).err().unwrap(),
        SessionError::UnsafeStorage
    );
    assert_eq!(
        fs::metadata(&database).unwrap().permissions().mode() & 0o7777,
        0o644
    );
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    execute(&h.h.manager, "PRAGMA user_version=99");
    assert_eq!(
        open(config(3600, 7200)).err().unwrap(),
        SessionError::UnsupportedSchema
    );
    assert_eq!(scalar(&h.h.manager, "PRAGMA user_version"), 99);
    execute(
        &h.h.manager,
        "PRAGMA user_version=4;UPDATE totp_factor SET intent_receipt='{}'",
    );
    assert_eq!(
        open(config(3600, 7200)).err().unwrap(),
        SessionError::CorruptStore
    );
}
