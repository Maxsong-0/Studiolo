//! 跨库失败不会授予 pending；安全吊销在审计故障下仍持久生效。

use super::*;
use std::sync::{Arc, atomic::Ordering};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase};

#[test]
fn intent_failure_has_no_session_and_outcome_failure_stays_pending_after_reopen() {
    let h = Harness::new(10, 100);
    h.audit.fail_intent.store(true, Ordering::SeqCst);
    assert_eq!(
        h.manager.issue(&identity()).unwrap_err(),
        SessionError::AuditUnavailable
    );
    assert_eq!(
        scalar(&h.manager, "SELECT total FROM auth_denials WHERE kind=6"),
        1
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 0);
    h.audit.fail_intent.store(false, Ordering::SeqCst);
    h.audit.fail_outcome.store(true, Ordering::SeqCst);
    assert_eq!(
        h.manager.issue(&identity()).unwrap_err(),
        SessionError::AuditUnavailable
    );
    assert_eq!(
        scalar(&h.manager, "SELECT total FROM auth_denials WHERE kind=6"),
        2
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='pending'"
        ),
        1
    );
    let reopened = h.reopen(10, 100);
    h.audit.fail_outcome.store(false, Ordering::SeqCst);
    reopened.flush_audit(256).unwrap();
    assert_eq!(
        scalar(
            &reopened,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        0
    );
    h.clock.set(1_100_000);
    let reopened = h.reopen(10, 100);
    assert_eq!(
        scalar(
            &reopened,
            "SELECT count(*) FROM sessions WHERE state='abandoned'"
        ),
        1
    );
}

#[test]
fn ambiguous_audit_append_is_exactly_replayed_without_activation() {
    let h = Harness::new(10, 100);
    let audit = h.audit.clone();
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.issue" && event.phase() == AuditPhase::Outcome {
            audit.after_append.store(true, Ordering::SeqCst);
        }
    }));
    assert_eq!(
        h.manager.issue(&identity()).unwrap_err(),
        SessionError::AuditUnavailable
    );
    *h.audit.hook.lock().unwrap() = None;
    h.audit.after_append.store(false, Ordering::SeqCst);
    let filter = AuditFilter::default().with_action("session.issue");
    let before = h.audit.query(&filter).unwrap();
    h.manager.flush_audit(256).unwrap();
    assert_eq!(h.audit.query(&filter).unwrap(), before);
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        0
    );
}

#[test]
fn audit_delay_crossing_deadline_never_releases_credentials() {
    let h = Harness::new(10, 100);
    let clock = h.clock.clone();
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.phase() == AuditPhase::Outcome {
            clock.set(1_100_000);
        }
    }));
    assert_eq!(
        h.manager.issue(&identity()).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        0
    );
}

#[test]
fn denial_flood_does_not_consume_revocation_budget_and_retry_never_reexecutes() {
    let h = Harness::new(10, 100);
    let issued = h.manager.issue(&identity()).unwrap();
    h.audit.fail.store(true, Ordering::SeqCst);
    let unknown = SessionToken::from_hex("e".repeat(64)).unwrap();
    for _ in 0..100 {
        assert_eq!(
            h.manager.check(&identity(), &unknown).unwrap_err(),
            SessionError::Rejected
        );
    }
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM audit_outbox WHERE bucket IS NOT NULL"
        ),
        1
    );
    assert_eq!(
        h.manager
            .revoke_all(&identity(), &issued.token, &issued.csrf)
            .unwrap()
            .revoked,
        1
    );
    assert_eq!(scalar(&h.manager, "SELECT epoch FROM auth_meta"), 2);
    let reopened = h.reopen(10, 100);
    assert_eq!(
        reopened.check(&identity(), &issued.token).unwrap_err(),
        SessionError::Rejected
    );
    h.audit.fail.store(false, Ordering::SeqCst);
    reopened.flush_audit(256).unwrap();
    reopened.flush_audit(256).unwrap();
    assert_eq!(scalar(&reopened, "SELECT epoch FROM auth_meta"), 2);
    assert_eq!(
        scalar(&reopened, "SELECT delivered FROM auth_denials WHERE kind=2"),
        101
    );
}

#[test]
fn independent_stores_have_distinct_audit_counter_namespaces() {
    let first = Harness::new(10, 100);
    let dir = directory();
    let random = Arc::new(TestRandom::default());
    random.next.store(12345, Ordering::SeqCst);
    let second = SessionManager::open(
        &dir.path().canonicalize().unwrap(),
        config(10, 100),
        first.audit.clone(),
        first.clock.clone(),
        random,
    )
    .unwrap();
    let bad = SessionToken::from_hex("f".repeat(64)).unwrap();
    for manager in [&*first.manager, &second] {
        for _ in 0..2 {
            assert_eq!(
                manager.check(&identity(), &bad).unwrap_err(),
                SessionError::Rejected
            );
            manager.flush_audit(256).unwrap();
        }
    }
    let all = first.audit.query(&AuditFilter::default()).unwrap();
    assert_eq!(all.len(), 4);
    assert_ne!(all[0].event().event_id(), all[2].event().event_id());
}
