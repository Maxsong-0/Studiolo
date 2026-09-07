//! 专用签发门槛、Cookie 内部秘密配对和当前会话撤权，不依赖 HTTP 或身份伪 DTO。

use super::http_support::*;
use super::*;
use std::sync::atomic::Ordering;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn enrolled_issue_rejects_unregistered_v2_v3_v4_without_changing_plain_issue() {
    for version in [2, 3, 4] {
        let h = fixture(version, 10, 100);
        h.clock.set(START + 1_000);
        assert_eq!(
            h.manager.issue_enrolled(&identity()).unwrap_err(),
            SessionError::FactorUnavailable
        );
        assert_eq!(
            scalar(&h.manager, "SELECT clock_floor FROM auth_meta"),
            START + 1_000
        );
        assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 0);
        assert_eq!(
            scalar(&h.manager, "SELECT next_issuance FROM credential_state"),
            1
        );
        assert_eq!(
            scalar(&h.manager, "SELECT total FROM auth_denials WHERE kind=6"),
            1
        );
        assert!(
            h.audit
                .query(&AuditFilter::default().with_action("session.issue"))
                .unwrap()
                .is_empty()
        );
        let issued = h.manager.issue(&identity()).unwrap();
        assert_eq!(
            h.manager.check(&identity(), &issued.token).unwrap().id,
            issued.session.id
        );
    }
}

#[test]
fn pending_enrollment_and_delivered_but_unactivated_proof_cannot_issue() {
    for version in [3, 4] {
        let h = fixture(version, 10, 100);
        let enrollment = enrollment(&h);
        h.audit.fail_outcome.store(true, Ordering::SeqCst);
        assert_eq!(
            enrollment.enroll(code()).unwrap_err(),
            SessionError::AuditUnavailable
        );
        assert_eq!(
            h.manager.issue_enrolled(&identity()).unwrap_err(),
            SessionError::FactorUnavailable
        );
        h.audit.fail_outcome.store(false, Ordering::SeqCst);
        let (totp, _) = Totp::attach(h.manager.clone(), seed()).unwrap();
        totp.flush_audit().unwrap();
        let reopened = h.reopen(10, 100);
        assert_eq!(
            reopened.issue_enrolled(&identity()).unwrap_err(),
            SessionError::FactorUnavailable
        );
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM sessions"), 0);
    }
}

#[test]
fn confirmed_registration_survives_reopen_and_issues_without_seed_handle() {
    for version in [3, 4] {
        let h = enrolled(version, 10, 100);
        let reopened = h.reopen(10, 100);
        let issued = reopened.issue_enrolled(&identity()).unwrap();
        assert_eq!(issued.session.state, SessionState::Active);
        assert_eq!(
            reopened
                .check_with_csrf(&identity(), &issued.token, &issued.csrf)
                .unwrap(),
            issued.session
        );
    }
}

#[test]
fn enrolled_issue_rejects_wrong_owner_and_expired_access_before_allocating() {
    let h = enrolled(4, 10, 100);
    for invalid in [
        crate::access::test_identity_with("another-person", 4_102_444_800),
        crate::access::test_identity_with("fictional-person", 1000),
    ] {
        assert_eq!(
            h.manager.issue_enrolled(&invalid).unwrap_err(),
            SessionError::Rejected
        );
    }
    assert_eq!(
        scalar(&h.manager, "SELECT next_issuance FROM credential_state"),
        1
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 0);
}

#[test]
fn paired_read_rejects_crossed_csrf_and_current_revoke_targets_only_authorized_row() {
    let h = enrolled(4, 10, 100);
    let first = h.manager.issue_enrolled(&identity()).unwrap();
    let second = h.manager.issue_enrolled(&identity()).unwrap();
    for (token, csrf) in [(&first.token, &second.csrf), (&second.token, &first.csrf)] {
        assert_eq!(
            h.manager
                .check_with_csrf(&identity(), token, csrf)
                .unwrap_err(),
            SessionError::Rejected
        );
        assert_eq!(
            h.manager
                .revoke_current(&identity(), token, csrf)
                .unwrap_err(),
            SessionError::Rejected
        );
    }
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        2
    );
    assert_eq!(
        h.manager
            .revoke_current(&identity(), &first.token, &first.csrf)
            .unwrap()
            .revoked,
        1
    );
    assert_eq!(
        h.manager
            .check_with_csrf(&identity(), &first.token, &first.csrf)
            .unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        h.manager
            .check_with_csrf(&identity(), &second.token, &second.csrf)
            .unwrap()
            .id,
        second.session.id
    );
    assert_eq!(scalar(&h.manager, "SELECT epoch FROM auth_meta"), 1);
}

#[test]
fn paired_read_and_current_revoke_remain_available_when_audit_is_down() {
    let h = enrolled(4, 10, 100);
    let issued = h.manager.issue_enrolled(&identity()).unwrap();
    h.audit.fail.store(true, Ordering::SeqCst);
    h.manager
        .check_with_csrf(&identity(), &issued.token, &issued.csrf)
        .unwrap();
    assert_eq!(
        h.manager
            .revoke_current(&identity(), &issued.token, &issued.csrf)
            .unwrap()
            .revoked,
        1
    );
    let reopened = h.reopen(10, 100);
    assert_eq!(
        reopened
            .check_with_csrf(&identity(), &issued.token, &issued.csrf)
            .unwrap_err(),
        SessionError::Rejected
    );
    h.audit.fail.store(false, Ordering::SeqCst);
    reopened.flush_audit(256).unwrap();
    assert_eq!(
        scalar(
            &reopened,
            "SELECT count(*) FROM sessions WHERE state='revoked'"
        ),
        1
    );
}
