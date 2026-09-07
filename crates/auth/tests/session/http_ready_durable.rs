//! 真实双 SQLite 全句柄关闭后重开；配错合法空账本时不补写登记历史。

use super::http_support::{START, code, seed};
use super::*;
use std::sync::{Arc, atomic::AtomicI64};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};

#[test]
fn readiness_checks_original_receipts_after_both_sqlite_stores_reopen() {
    let auth_dir = directory();
    let audit_dir = directory();
    let wrong_dir = directory();
    let auth_root = auth_dir.path().canonicalize().unwrap();
    let audit_root = audit_dir.path().canonicalize().unwrap();
    let wrong_root = wrong_dir.path().canonicalize().unwrap();
    let clock = Arc::new(TestClock(AtomicI64::new(START)));
    let random = Arc::new(TestRandom::default());
    let audit = Arc::new(SqliteAuditLedger::open(&audit_root).unwrap());
    let manager = Arc::new(
        SessionManager::migrate_v3(
            &auth_root,
            config(10, 100),
            audit.clone(),
            clock.clone(),
            random.clone(),
        )
        .unwrap(),
    );
    {
        let (_, enrollment) = Totp::attach(manager.clone(), seed()).unwrap();
        enrollment.enroll(code()).unwrap();
    }
    manager.require_http_ready().unwrap();
    let original = audit.query(&AuditFilter::default()).unwrap();
    assert_eq!(original.len(), 2);
    drop(manager);
    drop(audit);

    let audit = Arc::new(SqliteAuditLedger::open_existing(&audit_root).unwrap());
    let manager = SessionManager::open_existing(
        &auth_root,
        config(10, 100),
        audit.clone(),
        clock.clone(),
        random.clone(),
    )
    .unwrap();
    manager.require_http_ready().unwrap();
    assert_eq!(audit.query(&AuditFilter::default()).unwrap(), original);
    drop(manager);

    let wrong = Arc::new(SqliteAuditLedger::open(&wrong_root).unwrap());
    let manager =
        SessionManager::open_existing(&auth_root, config(10, 100), wrong.clone(), clock, random)
            .unwrap();
    assert_eq!(
        manager.require_http_ready(),
        Err(SessionError::AuditUnavailable)
    );
    assert!(wrong.query(&AuditFilter::default()).unwrap().is_empty());
    assert_eq!(scalar(&manager, "SELECT count(*) FROM sessions"), 0);
    assert_eq!(audit.query(&AuditFilter::default()).unwrap(), original);
}
