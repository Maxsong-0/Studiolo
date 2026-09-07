//! 显式 v3→v4，不借迁移登记因子、发码、提权或破坏原会话。

use super::*;
use std::sync::Arc;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn v3_open_stays_v3_and_explicit_upgrade_preserves_live_session_and_l3() {
    let h = Harness::new(3600, 7200);
    let manager = Arc::new(
        SessionManager::migrate_v2(
            &h.root,
            config(3600, 7200),
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone(),
        )
        .unwrap(),
    );
    let (totp, enrollment) = Totp::attach(manager.clone(), seed(OLD)).unwrap();
    enrollment.enroll(otp(&OLD, START)).unwrap();
    h.clock.set(NEXT);
    let owner = identity();
    let issued = manager.issue(&owner).unwrap();
    totp.step_up(&request(&owner, &issued, 1), otp(&OLD, NEXT))
        .unwrap();
    let opened = Arc::new(h.reopen(3600, 7200));
    assert_eq!(scalar(&opened, "PRAGMA user_version"), 3);
    assert_eq!(
        Recovery::attach(opened).err().unwrap(),
        SessionError::MigrationRequired
    );
    let before = h.audit.query(&AuditFilter::default()).unwrap();
    let upgraded = Arc::new(
        SessionManager::migrate_v3(
            &h.root,
            config(3600, 7200),
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone(),
        )
        .unwrap(),
    );
    assert_eq!(scalar(&upgraded, "PRAGMA user_version"), 4);
    assert_eq!(
        upgraded.check(&owner, &issued.token).unwrap().id,
        issued.session.id
    );
    assert_eq!(h.audit.query(&AuditFilter::default()).unwrap(), before);
    assert_eq!(scalar(&upgraded, "SELECT count(*) FROM recovery_codes"), 0);
    assert_eq!(
        scalar(&upgraded, "SELECT count(*) FROM recovery_pending"),
        0
    );
    let (totp, _) = Totp::attach(upgraded.clone(), seed(OLD)).unwrap();
    totp.require_l3(
        &request(&owner, &issued, 1),
        SensitiveAction::ManageRecoveryCodes,
    )
    .unwrap();
    Recovery::attach(upgraded)
        .unwrap()
        .rotate_codes(&request(&owner, &issued, 1))
        .unwrap();
    assert_eq!(scalar(&h.reopen(3600, 7200), "PRAGMA user_version"), 4);
}

#[test]
fn fresh_explicit_upgrade_reaches_v4_but_existing_v2_must_follow_the_sequence() {
    let h = Harness::new(3600, 7200);
    let old = h.manager.issue(&identity()).unwrap();
    assert_eq!(
        SessionManager::migrate_v3(
            &h.root,
            config(3600, 7200),
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone()
        )
        .err()
        .unwrap(),
        SessionError::MigrationRequired
    );
    assert_eq!(scalar(&h.manager, "PRAGMA user_version"), 2);
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sqlite_schema WHERE name='recovery_control'"
        ),
        0
    );
    h.manager.check(&identity(), &old.token).unwrap();
    let empty = directory();
    let root = empty.path().canonicalize().unwrap();
    let fresh = Arc::new(
        SessionManager::migrate_v3(
            &root,
            config(3600, 7200),
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone(),
        )
        .unwrap(),
    );
    assert_eq!(scalar(&fresh, "PRAGMA user_version"), 4);
    assert_eq!(scalar(&fresh, "SELECT count(*) FROM totp_factor"), 0);
    assert_eq!(scalar(&fresh, "SELECT count(*) FROM recovery_codes"), 0);
    Recovery::attach(fresh).unwrap();
}

#[test]
fn migration_does_not_repair_unknown_schema_or_corrupt_existing_metadata() {
    for sql in [
        "PRAGMA user_version=99",
        "UPDATE auth_meta SET config='{}'",
        "CREATE TABLE unexpected(value TEXT)",
    ] {
        let h = Harness::new(3600, 7200);
        SessionManager::migrate_v2(
            &h.root,
            config(3600, 7200),
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone(),
        )
        .unwrap();
        execute(&h.manager, sql);
        assert!(
            SessionManager::migrate_v3(
                &h.root,
                config(3600, 7200),
                h.audit.clone(),
                h.clock.clone(),
                h.random.clone()
            )
            .is_err()
        );
        assert_eq!(
            scalar(
                &h.manager,
                "SELECT count(*) FROM sqlite_schema WHERE name='recovery_control'"
            ),
            0
        );
    }
}
