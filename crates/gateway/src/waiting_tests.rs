//! 等待后撤权回归：确定进入真实等待后修改权威，验证准入、审计与槽所有权。

mod audit;
mod support;

use crate::GatewayError;
use std::sync::atomic::Ordering;
use support::{Admission, Branch, Fixture, Revocation};

#[derive(Clone, Copy)]
enum AuditFailure {
    None,
    Unavailable,
    Conflict,
}

fn revoked_waiter(
    branch: Branch,
    admission: Admission,
    reason: Revocation,
    audit_failure: AuditFailure,
) {
    let fixture = Fixture::new(branch);
    let first = fixture.spawn(false);
    fixture.audit.wait_first();
    let observed = fixture.arm_waiter();
    let second = fixture.spawn(matches!(admission, Admission::Conflict));
    fixture.wait_entered(observed);
    let denied = fixture.revoke(reason);
    fixture.audit.denial_failure.store(
        matches!(audit_failure, AuditFailure::Unavailable),
        Ordering::SeqCst,
    );
    fixture.audit.denial_conflict.store(
        matches!(audit_failure, AuditFailure::Conflict),
        Ordering::SeqCst,
    );
    fixture
        .audit
        .release_first(!matches!(admission, Admission::Proceed));
    let first = first.join().unwrap();
    let second = second.join().unwrap();
    match admission {
        Admission::Proceed => assert_eq!(first, Err(GatewayError::AuditUnavailable)),
        _ => assert_eq!(first, Ok(())),
    }
    let expected = match audit_failure {
        AuditFailure::None => denied.clone(),
        AuditFailure::Unavailable => GatewayError::AuditUnavailable,
        AuditFailure::Conflict => GatewayError::CallConflict,
    };
    assert_eq!(
        second,
        Err(expected),
        "{branch:?} {admission:?} released a waiter after {reason:?}"
    );
    fixture
        .audit
        .assert_denial(&denied, !matches!(audit_failure, AuditFailure::None));
    assert_eq!(
        fixture.audit.intents.load(Ordering::SeqCst),
        1,
        "waiter executed before reauthorization"
    );
    fixture.assert_slot(admission);
    fixture.restore();
    fixture.audit.denial_failure.store(false, Ordering::SeqCst);
    fixture.audit.denial_conflict.store(false, Ordering::SeqCst);
    // Proceed 的槽应可重新占用；Replay/Conflict 拒绝必须保留 A 的完成结果。
    assert_eq!(fixture.call(false), Ok(()));
    assert_eq!(
        fixture.audit.intents.load(Ordering::SeqCst),
        if matches!(admission, Admission::Proceed) {
            2
        } else {
            1
        }
    );
}

macro_rules! denial_cases {
    ($name:ident, $branch:ident, $admission:ident) => {
        #[test]
        fn $name() {
            for reason in [
                Revocation::Cancelled,
                Revocation::Fence,
                Revocation::Expired,
                Revocation::Unavailable,
                Revocation::Unknown,
            ] {
                revoked_waiter(
                    Branch::$branch,
                    Admission::$admission,
                    reason,
                    AuditFailure::None,
                );
            }
        }
    };
}

denial_cases!(invoke_replay_rechecks_waiting_authority, Invoke, Replay);
denial_cases!(relay_replay_rechecks_waiting_authority, Relay, Replay);
denial_cases!(
    invoke_inherited_proceed_rechecks_and_releases_slot,
    Invoke,
    Proceed
);
denial_cases!(
    relay_inherited_proceed_rechecks_and_releases_slot,
    Relay,
    Proceed
);
denial_cases!(
    invoke_conflict_prefers_current_revocation_reason,
    Invoke,
    Conflict
);
denial_cases!(
    relay_conflict_prefers_current_revocation_reason,
    Relay,
    Conflict
);

#[test]
fn denial_audit_failure_still_releases_only_the_waiters_own_slot() {
    for branch in [Branch::Invoke, Branch::Relay] {
        for admission in [Admission::Replay, Admission::Proceed, Admission::Conflict] {
            revoked_waiter(
                branch,
                admission,
                Revocation::Cancelled,
                AuditFailure::Unavailable,
            );
        }
    }
}

#[test]
fn denial_audit_conflict_is_not_misclassified_as_a_slot_conflict() {
    for branch in [Branch::Invoke, Branch::Relay] {
        for admission in [Admission::Replay, Admission::Proceed, Admission::Conflict] {
            revoked_waiter(
                branch,
                admission,
                Revocation::Cancelled,
                AuditFailure::Conflict,
            );
        }
    }
}

#[test]
fn deterministic_fixture_reaches_valid_replay_and_inherited_proceed() {
    for branch in [Branch::Invoke, Branch::Relay] {
        for proceed in [false, true] {
            let fixture = Fixture::new(branch);
            let first = fixture.spawn(false);
            fixture.audit.wait_first();
            let observed = fixture.arm_waiter();
            let second = fixture.spawn(false);
            fixture.wait_entered(observed);
            fixture.audit.release_first(!proceed);
            assert_eq!(
                first.join().unwrap(),
                if proceed {
                    Err(GatewayError::AuditUnavailable)
                } else {
                    Ok(())
                }
            );
            assert_eq!(second.join().unwrap(), Ok(()));
            assert_eq!(
                fixture.audit.intents.load(Ordering::SeqCst),
                if proceed { 2 } else { 1 }
            );
        }
    }
}
