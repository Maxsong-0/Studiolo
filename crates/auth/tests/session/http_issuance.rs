//! 跨库边界的确定性交错；审计回调不持 Auth 写锁，不使用 sleep 或脆弱时序。

use super::http_support::*;
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use studiolo_audit_ledger::AuditPhase;

#[test]
fn losing_confirmed_factor_during_intent_or_outcome_never_activates() {
    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
        let h = enrolled(4, 10, 100);
        let weak = Arc::downgrade(&h.manager);
        *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
            if event.action() == "session.issue" && event.phase() == phase {
                // 模拟已观察登记与后续事务之间的状态变化；剩余空因子结构仍合法。
                weak.upgrade()
                    .unwrap()
                    .database
                    .connection
                    .lock()
                    .unwrap()
                    .execute("DELETE FROM totp_factor", [])
                    .unwrap();
            }
        }));
        assert_eq!(
            h.manager.issue_enrolled(&identity()).unwrap_err(),
            SessionError::FactorUnavailable
        );
        *h.audit.hook.lock().unwrap() = None;
        let reopened = h.reopen(10, 100);
        reopened.flush_audit(256).unwrap();
        assert_eq!(
            scalar(
                &reopened,
                "SELECT count(*) FROM sessions WHERE state='active'"
            ),
            0
        );
        assert_eq!(
            scalar(&reopened, "SELECT count(*) FROM sessions"),
            i64::from(phase == AuditPhase::Outcome)
        );
        assert_eq!(
            reopened.issue_enrolled(&identity()).unwrap_err(),
            SessionError::FactorUnavailable
        );
    }
}

#[test]
fn confirmed_factor_receipt_corruption_is_not_treated_as_registered() {
    let h = enrolled(4, 10, 100);
    h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE totp_factor SET intent_receipt='{}'", [])
        .unwrap();
    assert_eq!(
        h.manager.issue_enrolled(&identity()).unwrap_err(),
        SessionError::CorruptStore
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 0);
    assert_eq!(
        scalar(&h.manager, "SELECT next_issuance FROM credential_state"),
        1
    );
}

#[test]
fn enrolled_issue_audit_failure_never_publishes_and_flush_cannot_activate() {
    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
        let h = enrolled(4, 10, 100);
        h.audit
            .fail_intent
            .store(phase == AuditPhase::Intent, Ordering::SeqCst);
        h.audit
            .fail_outcome
            .store(phase == AuditPhase::Outcome, Ordering::SeqCst);
        assert_eq!(
            h.manager.issue_enrolled(&identity()).unwrap_err(),
            SessionError::AuditUnavailable
        );
        assert_eq!(
            scalar(&h.manager, "SELECT total FROM auth_denials WHERE kind=6"),
            1
        );
        h.audit.fail_intent.store(false, Ordering::SeqCst);
        h.audit.fail_outcome.store(false, Ordering::SeqCst);
        let reopened = h.reopen(10, 100);
        reopened.flush_audit(256).unwrap();
        assert_eq!(
            scalar(
                &reopened,
                "SELECT count(*) FROM sessions WHERE state='active'"
            ),
            0
        );
        assert_eq!(
            scalar(
                &reopened,
                "SELECT count(*) FROM sessions WHERE state='pending'"
            ),
            i64::from(phase == AuditPhase::Outcome)
        );
    }
}

#[test]
fn another_instance_revoke_all_during_outcome_wins_over_enrolled_issuer() {
    let h = enrolled(4, 10, 100);
    let caller = h.manager.issue_enrolled(&identity()).unwrap();
    let other = h.reopen(10, 100);
    let once = AtomicBool::new(true);
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.issue"
            && event.phase() == AuditPhase::Outcome
            && once.swap(false, Ordering::SeqCst)
        {
            assert_eq!(
                other
                    .revoke_all(&identity(), &caller.token, &caller.csrf)
                    .unwrap()
                    .revoked,
                2
            );
        }
    }));
    assert_eq!(
        h.manager.issue_enrolled(&identity()).unwrap_err(),
        SessionError::Rejected
    );
    *h.audit.hook.lock().unwrap() = None;
    let reopened = h.reopen(10, 100);
    reopened.flush_audit(256).unwrap();
    assert_eq!(scalar(&reopened, "SELECT epoch FROM auth_meta"), 2);
    assert_eq!(
        scalar(
            &reopened,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        0
    );
    assert_eq!(
        scalar(
            &reopened,
            "SELECT count(*) FROM sessions WHERE state='revoked'"
        ),
        2
    );
}

#[test]
fn enrolled_issuer_observes_final_access_expiry_and_persists_clock_floor() {
    let h = enrolled(4, 10, 100);
    let clock = h.clock.clone();
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.issue" && event.phase() == AuditPhase::Outcome {
            clock.set(START + 10_000);
        }
    }));
    let identity = crate::access::test_identity_with("fictional-person", 1010);
    assert_eq!(
        h.manager.issue_enrolled(&identity).unwrap_err(),
        SessionError::Rejected
    );
    *h.audit.hook.lock().unwrap() = None;
    assert_eq!(
        scalar(&h.manager, "SELECT clock_floor FROM auth_meta"),
        START + 10_000
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='abandoned'"
        ),
        1
    );
    h.clock.set(START);
    assert_eq!(
        h.manager.issue_enrolled(&identity).unwrap_err(),
        SessionError::ClockRollback
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        0
    );
}
