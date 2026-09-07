//! 显式迁移的公开契约：原子升级、旧凭证保活、待激活不复活和固定旧摘要。

use super::migration_fixture as fixture;
#[path = "migration_integrity.rs"]
mod integrity;

use super::*;
use fixture::LegacyFixture;
use std::sync::atomic::Ordering;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn fresh_open_is_v2_with_empty_frozen_history() {
    let h = Harness::new(10, 100);
    assert_eq!(scalar(&h.manager, "PRAGMA user_version"), 2);
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM auth_legacy"), 0);
    assert_eq!(
        scalar(&h.manager, "SELECT legacy_count FROM credential_state"),
        0
    );
    assert_eq!(
        scalar(&h.manager, "SELECT next_issuance FROM credential_state"),
        1
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM session_seeds"), 0);
}

#[test]
fn ordinary_open_reports_migration_without_changing_v1_or_consuming_randomness() {
    let old = LegacyFixture::new();
    let before = old.snapshot();
    let random = old.h.random.next.load(Ordering::SeqCst);
    let result = SessionManager::open(
        &old.h.root,
        config(10, 100),
        old.h.audit.clone(),
        old.h.clock.clone(),
        old.h.random.clone(),
    );
    assert_eq!(result.err().unwrap(), SessionError::MigrationRequired);
    assert_eq!(old.snapshot(), before);
    assert_eq!(scalar(&old.h.manager, "PRAGMA user_version"), 1);
    assert_eq!(
        scalar(&old.h.manager, "SELECT count(*) FROM sqlite_schema"),
        10
    );
    assert_eq!(old.h.random.next.load(Ordering::SeqCst), random);
}

#[test]
fn explicit_migration_preserves_original_rows_and_live_credentials_without_granting_pending() {
    let old = LegacyFixture::new();
    let before = old.snapshot();
    let audit = old.h.audit.query(&AuditFilter::default()).unwrap();
    let manager = old.migrate();
    assert_eq!(old.snapshot(), before);
    assert_eq!(old.h.audit.query(&AuditFilter::default()).unwrap(), audit);
    assert_eq!(scalar(&manager, "SELECT count(*) FROM auth_legacy"), 2);
    assert_eq!(
        scalar(&manager, "SELECT legacy_count FROM credential_state"),
        2
    );
    assert_eq!(scalar(&manager, "SELECT count(*) FROM session_seeds"), 0);
    assert_eq!(manager.check(&identity(), &old.token).unwrap().id, old.id);
    assert_eq!(
        manager.check(&identity(), &old.pending_token).unwrap_err(),
        SessionError::Rejected
    );
    // 补记会包括刚才的拒绝聚合；无论补记数量如何，pending 均不可激活。
    manager.flush_audit(256).unwrap();
    assert_eq!(
        manager.check(&identity(), &old.pending_token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM sessions WHERE state='pending'"
        ),
        1
    );
    let reopened = old.h.reopen(10, 100);
    assert_eq!(reopened.check(&identity(), &old.token).unwrap().id, old.id);
}

#[test]
fn migrating_an_existing_v2_store_is_idempotent_and_does_not_refreeze_history() {
    let old = LegacyFixture::new();
    let manager = old.migrate();
    let issued = manager.issue(&identity()).unwrap();
    let before = old.snapshot();
    let random = old.h.random.next.load(Ordering::SeqCst);
    let audit = old.h.audit.query(&AuditFilter::default()).unwrap();
    let reopened = old.migrate();
    assert_eq!(old.snapshot(), before);
    assert_eq!(old.h.random.next.load(Ordering::SeqCst), random);
    assert_eq!(old.h.audit.query(&AuditFilter::default()).unwrap(), audit);
    assert_eq!(
        scalar(&reopened, "SELECT legacy_count FROM credential_state"),
        2
    );
    assert_eq!(
        reopened.check(&identity(), &issued.token).unwrap().id,
        issued.session.id
    );
}

#[test]
fn dropping_the_upgrade_transaction_rolls_back_all_new_tables_and_version() {
    let old = LegacyFixture::new();
    let before = old.snapshot();
    {
        let mut connection = old.h.manager.database.connection.lock().unwrap();
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        super::super::migration::v1(&transaction, &config(10, 100).encoded().unwrap()).unwrap();
        assert_eq!(schema::version(&transaction).unwrap(), 2);
        // 模拟完成 SQL 升级但尚未提交即中断；SQLite 必须撤销全部附加状态。
    }
    assert_eq!(old.snapshot(), before);
    assert_eq!(scalar(&old.h.manager, "PRAGMA user_version"), 1);
    assert_eq!(
        scalar(&old.h.manager, "SELECT count(*) FROM sqlite_schema"),
        10
    );
    old.migrate().check(&identity(), &old.token).unwrap();
}

#[test]
fn clock_failure_after_schema_commit_leaves_valid_v2_for_idempotent_reopen() {
    let old = LegacyFixture::new();
    let audit = old.h.audit.query(&AuditFilter::default()).unwrap();
    old.h.clock.set(999_999);
    let result = SessionManager::migrate_v1(
        &old.h.root,
        config(10, 100),
        old.h.audit.clone(),
        old.h.clock.clone(),
        old.h.random.clone(),
    );
    assert_eq!(result.err().unwrap(), SessionError::ClockRollback);
    // 原子边界是格式升级事务；其后的运行检查失败不把已提交的 v2 降级。
    assert_eq!(scalar(&old.h.manager, "PRAGMA user_version"), 2);
    assert_eq!(
        scalar(&old.h.manager, "SELECT clock_floor FROM auth_meta"),
        1_000_000
    );
    assert_eq!(
        scalar(&old.h.manager, "SELECT legacy_count FROM credential_state"),
        2
    );
    assert_eq!(
        scalar(
            &old.h.manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &old.h.manager,
            "SELECT count(*) FROM sessions WHERE state='pending'"
        ),
        1
    );
    assert_eq!(old.h.audit.query(&AuditFilter::default()).unwrap(), audit);
    old.h.clock.set(1_000_000);
    let reopened = old.h.reopen(10, 100);
    assert_eq!(reopened.check(&identity(), &old.token).unwrap().id, old.id);
    assert_eq!(
        reopened.check(&identity(), &old.pending_token).unwrap_err(),
        SessionError::Rejected
    );
}

#[test]
fn issuance_reservations_are_burned_across_reopen_and_never_wrap() {
    let h = Harness::new(10, 100);
    assert_eq!(
        h.manager
            .database
            .transaction(credential_store::reserve)
            .unwrap(),
        1
    );
    let reopened = h.reopen(10, 100);
    assert_eq!(
        reopened
            .database
            .transaction(credential_store::reserve)
            .unwrap(),
        2
    );
    reopened
        .database
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE credential_state SET next_issuance=?1", [i64::MAX])
        .unwrap();
    assert_eq!(
        reopened
            .database
            .transaction(credential_store::reserve)
            .unwrap_err(),
        SessionError::Capacity
    );
    assert_eq!(
        scalar(&reopened, "SELECT next_issuance FROM credential_state"),
        i64::MAX
    );
    assert_eq!(scalar(&reopened, "SELECT count(*) FROM sessions"), 0);
}
