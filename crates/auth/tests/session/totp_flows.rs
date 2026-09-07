//! 登记不提权；已确认近期认证只对原会话和严格期限有效。

use super::*;

#[test]
fn enrollment_does_not_grant_l3_and_consumed_codes_cannot_cross_sessions_or_restart() {
    let h = TotpHarness::new(3600, 7200);
    let owner = identity();
    let first = h.h.manager.issue(&owner).unwrap();
    let second = h.h.manager.issue(&owner).unwrap();
    h.enrollment.enroll(code(START)).unwrap();
    assert_eq!(
        h.totp
            .require_l3(&request(&owner, &first, 1), SensitiveAction::ChangeVault)
            .unwrap_err(),
        SessionError::ReauthenticationRequired
    );
    assert_eq!(
        h.totp
            .step_up(&request(&owner, &first, 1), code(START))
            .unwrap_err(),
        SessionError::Rejected
    );
    h.h.clock.set(NEXT);
    let proof = h
        .totp
        .step_up(&request(&owner, &first, 1), code(NEXT))
        .unwrap();
    assert_eq!(proof.verified_at_ms, NEXT);
    assert_eq!(proof.expires_at_ms, NEXT + 300_000);
    for action in [
        SensitiveAction::ActivatePolicy,
        SensitiveAction::ChangeVault,
        SensitiveAction::ChangeBackup,
        SensitiveAction::ChangeSync,
        SensitiveAction::RestoreVault,
        SensitiveAction::ReleaseFreeze,
        SensitiveAction::TestCredential,
    ] {
        h.totp
            .require_l3(&request(&owner, &first, 1), action)
            .unwrap();
    }
    assert_eq!(
        h.totp
            .require_l3(&request(&owner, &second, 1), SensitiveAction::ChangeBackup)
            .unwrap_err(),
        SessionError::ReauthenticationRequired
    );
    assert_eq!(
        h.totp
            .step_up(&request(&owner, &second, 1), code(NEXT))
            .unwrap_err(),
        SessionError::Rejected
    );
    let (_, reopened, enrollment) = h.reopen();
    reopened
        .require_l3(&request(&owner, &first, 1), SensitiveAction::ChangeSync)
        .unwrap();
    assert_eq!(
        reopened
            .step_up(&request(&owner, &second, 1), code(NEXT))
            .unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        enrollment.enroll(code(NEXT + 30_000)).unwrap_err(),
        SessionError::AlreadyEnrolled
    );
}

#[test]
fn every_sensitive_check_revalidates_csrf_owner_session_and_clock() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let first = h.h.manager.issue(&owner).unwrap();
    let second = h.h.manager.issue(&owner).unwrap();
    h.totp
        .step_up(&request(&owner, &first, 1), code(NEXT))
        .unwrap();
    let wrong_owner = crate::access::test_identity_with("someone-else", 4_102_444_800);
    let wrong_csrf = TotpRequest {
        csrf: &second.csrf,
        ..request(&owner, &first, 1)
    };
    for invalid in [wrong_csrf, request(&wrong_owner, &first, 1)] {
        assert_eq!(
            h.totp
                .require_l3(&invalid, SensitiveAction::RestoreVault)
                .unwrap_err(),
            SessionError::Rejected
        );
        assert_eq!(
            h.totp.step_up(&invalid, code(NEXT + 30_000)).unwrap_err(),
            SessionError::Rejected
        );
    }
    h.h.clock.set(NEXT - 1);
    assert_eq!(
        h.totp
            .require_l3(&request(&owner, &first, 1), SensitiveAction::ReleaseFreeze)
            .unwrap_err(),
        SessionError::ClockRollback
    );
    h.h.clock.set(NEXT);
    h.h.manager
        .revoke(&owner, &second.token, &second.csrf, &first.session.id)
        .unwrap();
    assert_eq!(
        h.totp
            .require_l3(&request(&owner, &first, 1), SensitiveAction::TestCredential)
            .unwrap_err(),
        SessionError::Rejected
    );
}

#[test]
fn l3_deadline_is_exclusive_and_capped_by_session_and_current_access() {
    for (absolute, access_expiry, expected) in [
        (7200, 4_102_444_800, NEXT + 300_000),
        (120, 4_102_444_800, NEXT + 120_000),
        (7200, 1150, 1_150_000),
    ] {
        let h = TotpHarness::enrolled(60.min(absolute), absolute);
        let owner = identity();
        let issued = h.h.manager.issue(&owner).unwrap();
        let access = crate::access::test_identity_with("fictional-person", access_expiry);
        let proof = h
            .totp
            .step_up(&request(&access, &issued, 1), code(NEXT))
            .unwrap();
        assert_eq!(proof.expires_at_ms, expected);
        // 逐次合法读取续 idle，但不能延长既定 L3、原会话 absolute 或原 step-up Access 上界。
        let mut now = NEXT;
        while now + 30_000 < expected {
            now += 30_000;
            h.h.clock.set(now);
            h.h.manager.check(&owner, &issued.token).unwrap();
        }
        h.h.clock.set(expected - 1);
        h.totp
            .require_l3(
                &request(&owner, &issued, 1),
                SensitiveAction::ActivatePolicy,
            )
            .unwrap();
        h.h.clock.set(expected);
        assert!(
            h.totp
                .require_l3(
                    &request(&owner, &issued, 1),
                    SensitiveAction::ActivatePolicy
                )
                .is_err()
        );
    }
}
