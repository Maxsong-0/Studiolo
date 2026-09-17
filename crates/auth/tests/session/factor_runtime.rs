//! 运行装配的公开行为；复用真实存储及恢复夹具，不复制原子状态机测试。

use super::durable_support::DurableRecovery;
use super::*;
use std::sync::{Arc, atomic::Ordering};
use studiolo_audit_ledger::AuditPhase;

#[path = "factor_maintenance.rs"]
mod maintenance;

fn attach(manager: Arc<SessionManager>) -> FactorRuntime {
    FactorRuntime::attach(manager, seed(OLD), Some(seed(NEW))).unwrap()
}

#[test]
fn fixed_slots_reopen_before_and_after_recovery_without_reversing_the_candidate() {
    let h = DurableRecovery::new();
    let (issued, codes) = h.ready();
    drop(attach(h.manager.clone()));
    let h = h.reopen();
    let runtime = attach(h.manager.clone());
    h.clock.time.set(CHANGE);
    runtime
        .recover(
            &request(&identity(), &issued, 1),
            copy_code(&codes, 0),
            otp(&NEW, CHANGE),
        )
        .unwrap();
    runtime.flush_audit().unwrap();
    drop(runtime);
    let h = h.reopen();
    let runtime = attach(h.manager.clone());
    let owner = identity();
    assert_eq!(
        h.manager.check(&owner, &issued.token).unwrap_err(),
        SessionError::Rejected
    );
    let fresh = h.manager.issue(&owner).unwrap();
    let req = request(&owner, &fresh, 1);
    h.clock.time.set(CHANGE + 60_000);
    runtime.step_up(&req, otp(&NEW, CHANGE + 60_000)).unwrap();
    let codes = runtime.rotate_codes(&req).unwrap();
    assert_eq!(codes.codes().len(), 10);
    assert_eq!(
        runtime.recover(&req, copy_code(&codes, 0), otp(&OLD, CHANGE + 60_000)),
        Err(SessionError::FactorUnavailable)
    );
    assert!(Totp::attach(h.manager.clone(), seed(NEW)).is_ok());
}

#[test]
fn an_existing_runtime_follows_another_managers_replacement_without_replaying_it() {
    let h = RecoveryHarness::new(3600, 7200);
    let runtime = attach(h.h.manager.clone());
    let (issued, codes) = h.ready();
    let (other, recovery) = h.reopen();
    let prepared = PreparedTotpFactor::prepare(other, seed(NEW)).unwrap();
    h.h.clock.set(CHANGE);
    drop(
        recovery
            .replace_factor(
                &request(&identity(), &issued, 1),
                copy_code(&codes, 0),
                &prepared,
                otp(&NEW, CHANGE),
            )
            .unwrap(),
    );
    let owner = identity();
    let fresh = h.h.manager.issue(&owner).unwrap();
    let req = request(&owner, &fresh, 1);
    h.h.clock.set(CHANGE + 60_000);
    runtime.step_up(&req, otp(&NEW, CHANGE + 60_000)).unwrap();
    let codes = runtime.rotate_codes(&req).unwrap();
    assert_eq!(
        runtime.recover(&req, copy_code(&codes, 0), otp(&OLD, CHANGE + 60_000)),
        Err(SessionError::FactorUnavailable)
    );
    // 其他可信调用仍能另行装配因子；本运行配置绝不因此回退到旧槽。
    h.h.clock.set(CHANGE + 120_000);
    let backwards = PreparedTotpFactor::prepare(h.h.manager.clone(), seed(OLD)).unwrap();
    drop(
        h.recovery
            .replace_factor(
                &req,
                copy_code(&codes, 0),
                &backwards,
                otp(&OLD, CHANGE + 120_000),
            )
            .unwrap(),
    );
    let fresh = h.h.manager.issue(&owner).unwrap();
    let req = request(&owner, &fresh, 1);
    assert_eq!(
        runtime
            .step_up(&req, otp(&OLD, CHANGE + 120_000))
            .unwrap_err(),
        SessionError::Configuration
    );
}

#[test]
fn unknown_bindings_and_duplicate_secret_slots_fail_at_attachment() {
    let h = RecoveryHarness::new(3600, 7200);
    for (current, replacement) in [
        (seed(NEW), None),
        (seed(OLD), Some(seed(OLD))),
        (
            seed(OLD),
            Some(TotpSeed::new("test-only-old".into(), NEW).unwrap()),
        ),
        (
            seed(OLD),
            Some(TotpSeed::new("different-reference".into(), OLD).unwrap()),
        ),
        (
            TotpSeed::new("unknown-one".into(), [91; 20]).unwrap(),
            Some(seed(NEW)),
        ),
    ] {
        assert_eq!(
            FactorRuntime::attach(h.h.manager.clone(), current, replacement)
                .err()
                .unwrap(),
            SessionError::Configuration
        );
    }
    let blank = DurableRecovery::new();
    assert_eq!(
        FactorRuntime::attach(blank.manager.clone(), seed(OLD), Some(seed(NEW)))
            .err()
            .unwrap(),
        SessionError::FactorUnavailable
    );
    let legacy = Harness::new(3600, 7200);
    assert_eq!(
        FactorRuntime::attach(legacy.manager.clone(), seed(OLD), None)
            .err()
            .unwrap(),
        SessionError::MigrationRequired
    );
}

#[test]
fn no_candidate_refuses_recovery_without_consuming_a_valid_code() {
    let h = RecoveryHarness::new(3600, 7200);
    let runtime = FactorRuntime::attach(h.h.manager.clone(), seed(OLD), None).unwrap();
    let (issued, codes) = h.ready();
    h.h.clock.set(CHANGE);
    let owner = identity();
    let req = request(&owner, &issued, 1);
    assert_eq!(
        runtime.recover(&req, copy_code(&codes, 0), otp(&NEW, CHANGE)),
        Err(SessionError::FactorUnavailable)
    );
    attach(h.h.manager.clone())
        .recover(&req, copy_code(&codes, 0), otp(&NEW, CHANGE))
        .unwrap();
}

#[test]
fn wrong_otp_does_not_grant_l3_and_unknown_external_binding_is_not_selected() {
    let h = RecoveryHarness::new(3600, 7200);
    let runtime = attach(h.h.manager.clone());
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    let req = request(&owner, &issued, 1);
    let accepted = [NEXT - 30_000, NEXT, NEXT + 30_000].map(|now| otp_text(&OLD, now));
    let wrong = (0..10)
        .map(|n| format!("{n:06}"))
        .find(|code| !accepted.contains(code))
        .unwrap();
    assert_eq!(
        runtime
            .step_up(&req, TotpCode::parse(wrong).unwrap())
            .unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        runtime.rotate_codes(&req).err().unwrap(),
        SessionError::ReauthenticationRequired
    );
    runtime.step_up(&req, otp(&OLD, NEXT)).unwrap();
    let codes = runtime.rotate_codes(&req).unwrap();
    let unknown = TotpSeed::new("unknown-external".into(), [92; 20]).unwrap();
    let prepared = PreparedTotpFactor::prepare(h.h.manager.clone(), unknown).unwrap();
    h.h.clock.set(CHANGE);
    drop(
        h.recovery
            .replace_factor(
                &req,
                copy_code(&codes, 0),
                &prepared,
                otp(&[92; 20], CHANGE),
            )
            .unwrap(),
    );
    let fresh = h.h.manager.issue(&owner).unwrap();
    let req = request(&owner, &fresh, 1);
    assert_eq!(
        runtime.step_up(&req, otp(&NEW, CHANGE)).unwrap_err(),
        SessionError::Configuration
    );
    assert_eq!(
        runtime.rotate_codes(&req).err().unwrap(),
        SessionError::Configuration
    );
}

#[test]
fn failed_audit_and_flush_do_not_switch_the_runtime_or_replay_consumed_inputs() {
    let h = RecoveryHarness::new(3600, 7200);
    let runtime = attach(h.h.manager.clone());
    let owner = identity();
    let (issued, codes) = h.ready();
    let req = request(&owner, &issued, 1);
    h.h.clock.set(CHANGE);
    let audit = Arc::downgrade(&h.h.audit);
    *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "totp.recovered" && event.phase() == AuditPhase::Outcome {
            audit
                .upgrade()
                .unwrap()
                .fail_outcome
                .store(true, Ordering::SeqCst);
        }
    }));
    assert_eq!(
        runtime.recover(&req, copy_code(&codes, 0), otp(&NEW, CHANGE)),
        Err(SessionError::AuditUnavailable)
    );
    *h.h.audit.hook.lock().unwrap() = None;
    h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
    runtime.flush_audit().unwrap();
    h.h.manager.check(&owner, &issued.token).unwrap();
    h.h.clock.set(CHANGE + 300_000);
    runtime.flush_audit().unwrap();
    runtime.step_up(&req, otp(&OLD, CHANGE + 300_000)).unwrap();
    assert_eq!(
        runtime.recover(&req, copy_code(&codes, 0), otp(&NEW, CHANGE + 300_000)),
        Err(SessionError::Rejected)
    );
    h.h.clock.set(CHANGE + 330_000);
    runtime
        .recover(&req, copy_code(&codes, 1), otp(&NEW, CHANGE + 330_000))
        .unwrap();
}
