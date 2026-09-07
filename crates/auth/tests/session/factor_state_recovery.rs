//! 因子装配持续观察权威绑定；真实双库恢复后旧会话不取得混合状态。

use super::durable_support::DurableRecovery;
use super::*;
use std::sync::Arc;

#[test]
fn state_follows_another_instance_and_reopening_without_offering_the_old_slot() {
    let h = DurableRecovery::new();
    let (issued, codes) = h.ready();
    let owner = identity();
    let runtime = FactorRuntime::attach(h.manager.clone(), seed(OLD), Some(seed(NEW))).unwrap();
    let before = runtime.state(&owner, &issued.token, &issued.csrf).unwrap();
    assert!(before.recovery_candidate_available);
    assert!(before.recent_authentication.is_some());
    h.clock.time.set(CHANGE);
    {
        let other = Arc::new(
            SessionManager::open_existing(
                &h.auth_dir.path().canonicalize().unwrap(),
                config(3600, 7200),
                h.audit.clone(),
                h.clock.clone(),
                h.random.clone(),
            )
            .unwrap(),
        );
        let recovery = Recovery::attach(other.clone()).unwrap();
        let prepared = PreparedTotpFactor::prepare(other, seed(NEW)).unwrap();
        drop(
            recovery
                .replace_factor(
                    &request(&owner, &issued, 1),
                    copy_code(&codes, 0),
                    &prepared,
                    otp(&NEW, CHANGE),
                )
                .unwrap(),
        );
    }
    assert_eq!(
        runtime.state(&owner, &issued.token, &issued.csrf),
        Err(SessionError::Rejected)
    );
    let fresh = h.manager.issue(&owner).unwrap();
    let after = runtime.state(&owner, &fresh.token, &fresh.csrf).unwrap();
    assert_eq!(after.observed_at_ms, CHANGE);
    assert!(!after.recovery_candidate_available);
    assert!(!after.pending);
    assert!(after.recent_authentication.is_none());
    drop(runtime);
    let h = h.reopen();
    let runtime = FactorRuntime::attach(h.manager.clone(), seed(OLD), Some(seed(NEW))).unwrap();
    assert_eq!(
        runtime.state(&owner, &fresh.token, &fresh.csrf).unwrap(),
        after
    );
    h.clock.time.set(CHANGE + 60_000);
    let recent = runtime
        .step_up(&request(&owner, &fresh, 1), otp(&NEW, CHANGE + 60_000))
        .unwrap();
    assert_eq!(
        runtime
            .state(&owner, &fresh.token, &fresh.csrf)
            .unwrap()
            .recent_authentication,
        Some(recent)
    );
}

#[test]
fn unknown_or_backwards_external_binding_fails_instead_of_reporting_unavailable_capability() {
    for backwards in [false, true] {
        let h = RecoveryHarness::new(3600, 7200);
        let runtime =
            FactorRuntime::attach(h.h.manager.clone(), seed(OLD), Some(seed(NEW))).unwrap();
        let owner = identity();
        let (issued, codes) = h.ready();
        h.h.clock.set(CHANGE);
        if backwards {
            runtime
                .recover(
                    &request(&owner, &issued, 1),
                    copy_code(&codes, 0),
                    otp(&NEW, CHANGE),
                )
                .unwrap();
            let fresh = h.h.manager.issue(&owner).unwrap();
            assert!(
                !runtime
                    .state(&owner, &fresh.token, &fresh.csrf)
                    .unwrap()
                    .recovery_candidate_available
            );
            h.h.clock.set(CHANGE + 60_000);
            let req = request(&owner, &fresh, 1);
            runtime.step_up(&req, otp(&NEW, CHANGE + 60_000)).unwrap();
            let codes = runtime.rotate_codes(&req).unwrap();
            h.h.clock.set(CHANGE + 120_000);
            let old = PreparedTotpFactor::prepare(h.h.manager.clone(), seed(OLD)).unwrap();
            drop(
                h.recovery
                    .replace_factor(
                        &req,
                        copy_code(&codes, 0),
                        &old,
                        otp(&OLD, CHANGE + 120_000),
                    )
                    .unwrap(),
            );
        } else {
            let unknown = PreparedTotpFactor::prepare(
                h.h.manager.clone(),
                TotpSeed::new("external-unknown".into(), [92; 20]).unwrap(),
            )
            .unwrap();
            drop(
                h.recovery
                    .replace_factor(
                        &request(&owner, &issued, 1),
                        copy_code(&codes, 0),
                        &unknown,
                        otp(&[92; 20], CHANGE),
                    )
                    .unwrap(),
            );
        }
        let fresh = h.h.manager.issue(&owner).unwrap();
        assert_eq!(
            runtime.state(&owner, &fresh.token, &fresh.csrf),
            Err(SessionError::Configuration)
        );
    }
}
