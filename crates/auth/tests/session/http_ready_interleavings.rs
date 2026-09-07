//! 查询期间 Auth 可继续运行；最终检查不缓存旧证明，错误仍提交观察地板。

use super::*;
use super::{http_ready_support::*, http_support::*};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn readiness_rejects_a_different_well_formed_proof_after_external_query() {
    let h = enrolled(4, 10, 100);
    let audit = Arc::new(ReadAudit::new(h.audit.clone()));
    let manager = open(&h, audit.clone());
    let weak = Arc::downgrade(&h.manager);
    let clock = h.clock.clone();
    let once = AtomicBool::new(true);
    *audit.hook.lock().unwrap() = Some(Arc::new(move || {
        if once.swap(false, Ordering::SeqCst) {
            clock.set(START + 1000);
            // 改成仍满足完整性规则的确认时刻，验证最终比较的是整份证明而非存在性。
            weak.upgrade()
                .unwrap()
                .run(|db, observed| {
                    db.execute("UPDATE totp_factor SET confirmed_at=?1", [observed.now])
                        .unwrap();
                    Ok(Ok(()))
                })
                .unwrap();
        }
    }));
    assert_eq!(manager.require_http_ready(), Err(SessionError::Rejected));
    assert_eq!(
        scalar(&manager, "SELECT clock_floor FROM auth_meta"),
        START + 1000
    );
    assert_eq!(audit.appends.load(Ordering::SeqCst), 0);
    *audit.hook.lock().unwrap() = None;
    h.reopen(10, 100).require_http_ready().unwrap();
}

#[test]
fn readiness_rejects_factor_disappearance_without_repairing_or_issuing() {
    let h = enrolled(4, 10, 100);
    let audit = Arc::new(ReadAudit::new(h.audit.clone()));
    let manager = open(&h, audit.clone());
    let weak = Arc::downgrade(&h.manager);
    *audit.hook.lock().unwrap() = Some(Arc::new(move || {
        weak.upgrade()
            .unwrap()
            .database
            .connection
            .lock()
            .unwrap()
            .execute("DELETE FROM totp_factor", [])
            .unwrap();
    }));
    assert_eq!(
        manager.require_http_ready(),
        Err(SessionError::FactorUnavailable)
    );
    assert_eq!(audit.appends.load(Ordering::SeqCst), 0);
    assert_eq!(scalar(&manager, "SELECT count(*) FROM sessions"), 0);
    assert_eq!(scalar(&manager, "SELECT count(*) FROM totp_factor"), 0);
}

#[test]
fn readiness_query_failure_after_expiry_commits_floor_and_cannot_revive_on_rollback() {
    let h = enrolled(4, 10, 100);
    let issued = h.manager.issue(&identity()).unwrap();
    let audit = Arc::new(ReadAudit::new(h.audit.clone()));
    audit.fault.store(Fault::Query as u8, Ordering::SeqCst);
    let manager = open(&h, audit.clone());
    let clock = h.clock.clone();
    *audit.hook.lock().unwrap() = Some(Arc::new(move || clock.set(START + 10_000)));
    assert_eq!(
        manager.require_http_ready(),
        Err(SessionError::AuditUnavailable)
    );
    assert_eq!(
        scalar(&manager, "SELECT clock_floor FROM auth_meta"),
        START + 10_000
    );
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
    assert_eq!(audit.appends.load(Ordering::SeqCst), 0);
    h.clock.set(START);
    assert_eq!(
        manager.check(&identity(), &issued.token).unwrap_err(),
        SessionError::ClockRollback
    );
}

#[test]
fn readiness_observes_expiry_without_flushing_outbox_or_reclaiming_terminal_rows() {
    let h = enrolled(4, 10, 100);
    h.manager.issue(&identity()).unwrap();
    let before = h.audit.query(&AuditFilter::default()).unwrap();
    let audit = Arc::new(ReadAudit::new(h.audit.clone()));
    let manager = open(&h, audit.clone());
    h.clock.set(START + 10_000);
    manager.require_http_ready().unwrap();
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM audit_outbox WHERE kind='expire' AND receipt_json IS NULL"
        ),
        1
    );
    assert_eq!(
        scalar(&manager, "SELECT count(*) FROM auth_retention_plan"),
        0
    );
    assert_eq!(
        scalar(&manager, "SELECT next_issuance FROM credential_state"),
        2
    );
    assert_eq!(audit.appends.load(Ordering::SeqCst), 0);
    assert_eq!(h.audit.query(&AuditFilter::default()).unwrap(), before);
}
