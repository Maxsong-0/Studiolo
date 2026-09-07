//! 到期点与时钟地板；localhost、旧 L3、坏 CSRF 均不是恢复授权。

use super::*;
use std::sync::Arc;
use studiolo_audit_ledger::AuditPhase;

#[test]
fn delayed_replacement_is_capped_by_window_access_and_session_absolute_deadline() {
    for (absolute, access_expiry, deadline) in [
        (7200, 4_102_444_800, CHANGE + 300_000),
        (7200, 1127, 1_127_000),
        (100, 4_102_444_800, NEXT + 100_000),
    ] {
        for delta in [-1, 0] {
            let h = RecoveryHarness::new(absolute.min(3600), absolute);
            let (issued, codes) = h.ready();
            let prepared = h.prepared();
            let owner = crate::access::test_identity_with("fictional-person", access_expiry);
            h.h.clock.set(CHANGE);
            let clock = h.h.clock.clone();
            *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
                if event.action() == "totp.recovered" && event.phase() == AuditPhase::Outcome {
                    clock.set(deadline + delta);
                }
            }));
            let result = h.recovery.replace_factor(
                &request(&owner, &issued, 1),
                copy_code(&codes, 0),
                &prepared,
                otp(&NEW, CHANGE),
            );
            assert_eq!(result.is_ok(), delta == -1);
            *h.h.audit.hook.lock().unwrap() = None;
            if delta == 0 {
                assert_eq!(
                    scalar(&h.h.manager, "SELECT used_mask FROM recovery_control"),
                    1
                );
                assert!(h.recovery.flush_audit().unwrap() <= 2);
                assert!(Totp::attach(h.h.manager.clone(), seed(OLD)).is_ok());
            }
        }
    }
}

#[test]
fn bad_identity_csrf_and_loopback_do_not_consume_or_replace_factor() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (issued, codes) = h.ready();
    let other = h.h.manager.issue(&owner).unwrap();
    let outsider = crate::access::test_identity_with("other-fictional-person", 4_102_444_800);
    let prepared = h.prepared();
    h.h.clock.set(CHANGE);
    let wrong_csrf = TotpRequest {
        csrf: &other.csrf,
        source: "127.0.0.1".parse().unwrap(),
        ..request(&owner, &issued, 1)
    };
    let wrong_owner = TotpRequest {
        source: "127.0.0.1".parse().unwrap(),
        ..request(&outsider, &issued, 1)
    };
    for req in [wrong_csrf, wrong_owner] {
        assert!(
            h.recovery
                .replace_factor(&req, copy_code(&codes, 0), &prepared, otp(&NEW, CHANGE))
                .is_err()
        );
        assert!(h.recovery.rotate_codes(&req).is_err());
    }
    assert_eq!(
        scalar(&h.h.manager, "SELECT used_mask FROM recovery_control"),
        0
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        0
    );
    h.h.clock.set(CHANGE - 1);
    assert_eq!(
        h.recovery
            .replace_factor(
                &request(&owner, &issued, 1),
                copy_code(&codes, 0),
                &prepared,
                otp(&NEW, CHANGE)
            )
            .err()
            .unwrap(),
        SessionError::ClockRollback
    );
}

#[test]
fn sequence_capacity_rejection_commits_observed_floor_and_expired_session() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (issued, codes) = h.ready();
    let short = crate::access::test_identity_with("fictional-person", 1065);
    let victim = h.h.manager.issue(&short).unwrap();
    let prepared = h.prepared();
    execute(
        &h.h.manager,
        "UPDATE recovery_control SET next_operation=9223372036854775807",
    );
    h.h.clock.set(CHANGE);
    assert_eq!(
        h.recovery
            .replace_factor(
                &request(&owner, &issued, 1),
                copy_code(&codes, 0),
                &prepared,
                otp(&NEW, CHANGE)
            )
            .err()
            .unwrap(),
        SessionError::Capacity
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT clock_floor FROM auth_meta"),
        CHANGE
    );
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT used_mask FROM recovery_control"),
        0
    );
    h.h.clock.set(NEXT);
    assert_eq!(
        h.h.manager.check(&owner, &victim.token).unwrap_err(),
        SessionError::ClockRollback
    );
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
}
