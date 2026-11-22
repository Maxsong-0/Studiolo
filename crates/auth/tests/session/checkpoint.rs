//! 两个真实数据库的截止点；Auth 不接纳缺 ACK、错账本或未确认的导出。
use super::*;
use std::{
    sync::atomic::AtomicI64,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::SqliteAuditLedger;
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
#[test]
fn auth_requires_real_audit_coverage_before_sealing() {
    let auth = directory();
    let audit = directory();
    let wrong_audit = directory();
    let target = directory();
    let ledger = Arc::new(SqliteAuditLedger::open(audit.path()).unwrap());
    let manager = SessionManager::open(
        auth.path(),
        config(10, 100),
        ledger.clone(),
        Arc::new(TestClock(AtomicI64::new(1_000_000))),
        Arc::new(TestRandom::default()),
    )
    .unwrap();
    manager.issue(&identity()).unwrap();
    let auth_source = manager.checkpoint_source().unwrap();
    let audit_source = ledger.checkpoint_source().unwrap();
    let _quiescence = ledger.quiesce_operations(deadline()).unwrap();
    let mut frozen_auth = auth_source.freeze(deadline()).unwrap();
    let frozen_audit = audit_source.freeze(deadline()).unwrap();
    // 校验了定格、还没核回执：照样不能导出。
    frozen_auth.validate().unwrap();
    assert!(frozen_auth.export(target.path()).is_err());
    let wrong = SqliteAuditLedger::open(wrong_audit.path()).unwrap();
    let wrong = wrong
        .checkpoint_source()
        .unwrap()
        .freeze(deadline())
        .unwrap();
    assert!(frozen_auth.verify_audit(&wrong).is_err());
    let coverage = frozen_auth.verify_audit(&frozen_audit).unwrap();
    assert!(coverage.checked() >= 2);
    assert_eq!(coverage.maximum_offset(), 2);
    frozen_auth.export(target.path()).unwrap();
    let copy = rusqlite::Connection::open(target.path().join("auth.sqlite3")).unwrap();
    assert_eq!(
        copy.query_row("SELECT state FROM sessions", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "active"
    );
    assert!(frozen_auth.verify_audit(&wrong).is_err());
    assert!(frozen_auth.export(directory().path()).is_err());
}
#[test]
fn receipts_checked_without_validating_the_frozen_moment_do_not_export() {
    let auth = directory();
    let audit = directory();
    let target = directory();
    let ledger = Arc::new(SqliteAuditLedger::open(audit.path()).unwrap());
    let manager = SessionManager::open(
        auth.path(),
        config(10, 100),
        ledger.clone(),
        Arc::new(TestClock(AtomicI64::new(1_000_000))),
        Arc::new(TestRandom::default()),
    )
    .unwrap();
    manager.issue(&identity()).unwrap();
    let auth_source = manager.checkpoint_source().unwrap();
    let audit_source = ledger.checkpoint_source().unwrap();
    let _quiescence = ledger.quiesce_operations(deadline()).unwrap();
    let mut frozen_auth = auth_source.freeze(deadline()).unwrap();
    let frozen_audit = audit_source.freeze(deadline()).unwrap();
    // 回执核过、定格没校验：水位只由放开之后的校验得出（BUG-045），不能导出，目标也不动；
    // 校验之后同一个定格、同一个目标导得出。
    assert!(frozen_auth.verify_audit(&frozen_audit).unwrap().checked() >= 2);
    assert!(frozen_auth.export(target.path()).is_err());
    frozen_auth.validate().unwrap();
    frozen_auth.export(target.path()).unwrap();
}
#[test]
fn pending_auth_does_not_count_as_drained_and_retry_is_possible() {
    let h = Harness::new(10, 100);
    h.audit
        .fail
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(h.manager.issue(&identity()).is_err());
    let source = h.manager.checkpoint_source().unwrap();
    // 冻结窗口里只定格（BUG-045）：没排空照样定得住，放开之后在定格上校验时拒绝。
    assert!(source.freeze(deadline()).unwrap().validate().is_err());
    h.audit
        .fail
        .store(false, std::sync::atomic::Ordering::SeqCst);
    h.manager.flush_audit(256).unwrap();
    // 补记完成并不自动把未交给用户的会话变成 active；它须经既有状态机终结。
    h.clock.set(1_200_000);
    h.manager.run(|_, _| Ok(Ok(()))).unwrap();
    h.manager.flush_audit(256).unwrap();
    let mut frozen = source.freeze(deadline()).unwrap();
    frozen.validate().unwrap();
    assert!(frozen.export(directory().path()).is_err());
}
