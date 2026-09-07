//! v3 只能显式升级；旧会话保持有效，但迁移不产生因子登记或 L3。

use super::super::migration_fixture as fixture;

use super::*;
use std::sync::Arc;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn existing_v2_requires_explicit_migration_and_preserves_sessions_without_elevation() {
    let h = Harness::new(3600, 7200);
    let owner = identity();
    let issued = h.manager.issue(&owner).unwrap();
    assert_eq!(scalar(&h.manager, "PRAGMA user_version"), 2);
    assert_eq!(
        Totp::attach(h.manager.clone(), seed()).err().unwrap(),
        SessionError::MigrationRequired
    );
    let before = h.audit.query(&AuditFilter::default()).unwrap();
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
    assert_eq!(scalar(&manager, "PRAGMA user_version"), 3);
    assert_eq!(
        manager.check(&owner, &issued.token).unwrap(),
        issued.session
    );
    assert_eq!(h.audit.query(&AuditFilter::default()).unwrap(), before);
    let (totp, _) = Totp::attach(manager, seed()).unwrap();
    assert!(
        totp.require_l3(&request(&owner, &issued, 1), SensitiveAction::ChangeVault)
            .is_err()
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM totp_factor"), 0);
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM totp_grants"), 0);
    let reopened = Arc::new(h.reopen(3600, 7200));
    assert_eq!(
        reopened.check(&owner, &issued.token).unwrap(),
        issued.session
    );
    Totp::attach(reopened, seed()).unwrap();
}

#[test]
fn v1_must_pass_the_existing_v2_migration_before_totp_upgrade() {
    let old = fixture::LegacyFixture::new();
    let before = old.snapshot();
    let migrate = || {
        SessionManager::migrate_v2(
            &old.h.root,
            config(10, 100),
            old.h.audit.clone(),
            old.h.clock.clone(),
            old.h.random.clone(),
        )
    };
    assert_eq!(migrate().err().unwrap(), SessionError::MigrationRequired);
    assert_eq!(old.snapshot(), before);
    assert_eq!(scalar(&old.h.manager, "PRAGMA user_version"), 1);
    old.migrate();
    let manager = Arc::new(migrate().unwrap());
    assert_eq!(scalar(&manager, "PRAGMA user_version"), 3);
    assert_eq!(manager.check(&identity(), &old.token).unwrap().id, old.id);
    assert_eq!(
        manager.check(&identity(), &old.pending_token).unwrap_err(),
        SessionError::Rejected
    );
    let (totp, _) = Totp::attach(manager.clone(), seed()).unwrap();
    let owner = identity();
    let req = TotpRequest {
        identity: &owner,
        token: &old.token,
        csrf: &old.csrf,
        source: "127.0.0.1".parse().unwrap(),
    };
    assert!(
        totp.require_l3(&req, SensitiveAction::ChangeBackup)
            .is_err()
    );
    assert!(totp.step_up(&req, code(START)).is_err());
    assert_eq!(scalar(&manager, "SELECT count(*) FROM totp_factor"), 0);
}

#[test]
fn unknown_or_corrupt_store_cannot_be_repaired_by_totp_migration() {
    for sql in [
        "PRAGMA user_version=99",
        "UPDATE auth_meta SET config='{}'",
        "CREATE TABLE unexpected(value TEXT)",
    ] {
        let h = Harness::new(3600, 7200);
        h.manager
            .database
            .connection
            .lock()
            .unwrap()
            .execute_batch(sql)
            .unwrap();
        assert!(
            SessionManager::migrate_v2(
                &h.root,
                config(3600, 7200),
                h.audit.clone(),
                h.clock.clone(),
                h.random.clone(),
            )
            .is_err()
        );
        assert_eq!(
            scalar(
                &h.manager,
                "SELECT count(*) FROM sqlite_schema WHERE name='totp_control'"
            ),
            0
        );
    }
}
