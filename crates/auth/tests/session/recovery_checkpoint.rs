//! v4 恢复 JSON 中的历史证明也逐条进入真实 Audit 覆盖，不能只扫 SQL 回执列。
use super::*;
use std::{
    sync::atomic::AtomicI64,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
#[test]
fn recovery_embedded_proofs_are_checked_against_the_frozen_original_ledger() {
    let auth = directory();
    let audit = directory();
    let target = directory();
    let ledger = Arc::new(SqliteAuditLedger::open(audit.path()).unwrap());
    let clock = Arc::new(TestClock(AtomicI64::new(START)));
    let manager = Arc::new(
        SessionManager::migrate_v3(
            auth.path(),
            config(3600, 7200),
            ledger.clone(),
            clock.clone(),
            Arc::new(TestRandom::default()),
        )
        .unwrap(),
    );
    let (totp, enrollment) = Totp::attach(manager.clone(), seed(OLD)).unwrap();
    enrollment.enroll(otp(&OLD, START)).unwrap();
    clock.set(NEXT);
    let owner = identity();
    let issued = manager.issue(&owner).unwrap();
    let req = request(&owner, &issued, 1);
    totp.step_up(&req, otp(&OLD, NEXT)).unwrap();
    Recovery::attach(manager.clone())
        .unwrap()
        .rotate_codes(&req)
        .unwrap();
    let count = ledger.query(&AuditFilter::default()).unwrap().len() as u64;
    let source = manager.checkpoint_source().unwrap();
    let audit_source = ledger.checkpoint_source().unwrap();
    let _quiet = ledger.quiesce_operations(deadline()).unwrap();
    let mut frozen = source.freeze(deadline()).unwrap();
    let audit = audit_source.freeze(deadline()).unwrap();
    let coverage = frozen.verify_audit(&audit).unwrap();
    assert!(coverage.checked() >= count);
    assert_eq!(coverage.maximum_offset(), count);
    frozen.export(target.path()).unwrap();
}
