//! 网页签发只触发一轮回收；原 issue 不维护，低流量期间也没有后台到期承诺。

use super::http_support::*;
use super::*;
use std::sync::{Arc, atomic::Ordering};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn enrolled_issue_observes_expiry_then_reclaims_only_one_sixteen_row_batch() {
    let h = enrolled(4, 10, 100);
    for _ in 0..20 {
        h.manager.issue(&identity()).unwrap();
    }
    h.clock.set(START + 10_000);
    let issued = h.manager.issue_enrolled(&identity()).unwrap();
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 5);
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        4
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        1
    );
    assert_eq!(
        h.audit
            .query(&AuditFilter::default().with_action("session.compacted"))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        h.manager
            .check_with_csrf(&identity(), &issued.token, &issued.csrf)
            .unwrap()
            .id,
        issued.session.id
    );
}

#[test]
fn plain_issue_observes_expiry_without_implicit_retention() {
    let h = enrolled(4, 10, 100);
    let first = h.manager.issue(&identity()).unwrap();
    h.clock.set(START + 10_000);
    h.manager.issue(&identity()).unwrap();
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 2);
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
    assert!(
        h.audit
            .query(&AuditFilter::default().with_action("session.compacted"))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        h.manager.check(&identity(), &first.token).unwrap_err(),
        SessionError::Rejected
    );
}

#[test]
fn maintenance_cannot_cache_registration_for_the_following_issue() {
    let h = enrolled(4, 10, 100);
    h.manager.issue(&identity()).unwrap();
    h.clock.set(START + 10_000);
    let weak = Arc::downgrade(&h.manager);
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.compacted" {
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
    assert_eq!(
        scalar(&reopened, "SELECT next_issuance FROM credential_state"),
        2
    );
    assert_eq!(scalar(&reopened, "SELECT count(*) FROM sessions"), 0);
}

#[test]
fn retention_audit_failure_refuses_new_issue_but_commits_observed_expiry() {
    let h = enrolled(4, 10, 100);
    let old = h.manager.issue(&identity()).unwrap();
    h.clock.set(START + 10_000);
    h.audit.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        h.manager.issue_enrolled(&identity()).unwrap_err(),
        SessionError::AuditUnavailable
    );
    assert_eq!(
        scalar(&h.manager, "SELECT next_issuance FROM credential_state"),
        2
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
    assert_eq!(
        scalar(&h.manager, "SELECT total FROM auth_denials WHERE kind=6"),
        1
    );
    assert_eq!(
        scalar(&h.manager, "SELECT clock_floor FROM auth_meta"),
        START + 10_000
    );
    h.clock.set(START);
    assert_eq!(
        h.manager
            .check_with_csrf(&identity(), &old.token, &old.csrf)
            .unwrap_err(),
        SessionError::ClockRollback
    );
    h.clock.set(START + 10_000);
    h.audit.fail.store(false, Ordering::SeqCst);
    let reopened = h.reopen(10, 100);
    reopened.issue_enrolled(&identity()).unwrap();
    assert_eq!(scalar(&reopened, "SELECT count(*) FROM sessions"), 1);
    assert_eq!(
        scalar(
            &reopened,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        1
    );
}
