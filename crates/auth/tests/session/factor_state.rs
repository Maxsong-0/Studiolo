//! 公开状态投影复用真实会话检查；期限显示不能绕过任何动作的即时L3检查。

use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn state_is_session_specific_and_uses_the_same_exact_deadline_as_both_l3_actions() {
    let h = RecoveryHarness::new(3600, 7200);
    let runtime = FactorRuntime::attach(h.h.manager.clone(), seed(OLD), Some(seed(NEW))).unwrap();
    let owner = identity();
    let first = h.h.manager.issue(&owner).unwrap();
    let second = h.h.manager.issue(&owner).unwrap();
    let req = request(&owner, &first, 1);
    let state = runtime.state(&owner, &first.token, &first.csrf).unwrap();
    assert_eq!(state.observed_at_ms, NEXT);
    assert!(state.recent_authentication.is_none());
    assert!(state.recovery_candidate_available);
    assert!(!state.pending);
    let recent = runtime.step_up(&req, otp(&OLD, NEXT)).unwrap();
    h.h.clock.set(recent.expires_at_ms - 1);
    assert_eq!(
        runtime
            .state(&owner, &first.token, &first.csrf)
            .unwrap()
            .recent_authentication,
        Some(recent)
    );
    assert!(
        runtime
            .state(&owner, &second.token, &second.csrf)
            .unwrap()
            .recent_authentication
            .is_none()
    );
    h.totp
        .require_l3(&req, SensitiveAction::ChangeVault)
        .unwrap();
    runtime.rotate_codes(&req).unwrap();
    h.h.clock.set(recent.expires_at_ms);
    assert!(
        runtime
            .state(&owner, &first.token, &first.csrf)
            .unwrap()
            .recent_authentication
            .is_none()
    );
    assert_eq!(
        h.totp.require_l3(&req, SensitiveAction::ChangeVault),
        Err(SessionError::ReauthenticationRequired)
    );
    assert_eq!(
        runtime.rotate_codes(&req).err().unwrap(),
        SessionError::ReauthenticationRequired
    );
}

struct CountingClock {
    time: Arc<TestClock>,
    reads: AtomicUsize,
}

impl SessionClock for CountingClock {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.time.now_unix_ms()
    }
}

#[test]
fn repeated_state_uses_one_observation_without_randomness_audit_or_verification_budget() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    let clock = Arc::new(CountingClock {
        time: h.h.clock.clone(),
        reads: AtomicUsize::new(0),
    });
    let manager = Arc::new(
        SessionManager::open_existing(
            &h.h.root,
            config(3600, 7200),
            h.h.audit.clone(),
            clock.clone(),
            h.h.random.clone(),
        )
        .unwrap(),
    );
    let runtime = FactorRuntime::attach(manager, seed(OLD), None).unwrap();
    let before = h.h.audit.query(&AuditFilter::default()).unwrap();
    clock.reads.store(0, Ordering::SeqCst);
    h.h.audit.fail.store(true, Ordering::SeqCst);
    h.h.random.fail.store(true, Ordering::SeqCst);
    for _ in 0..20 {
        let state = runtime.state(&owner, &issued.token, &issued.csrf).unwrap();
        assert_eq!(state.observed_at_ms, NEXT);
        assert!(!state.recovery_candidate_available);
        assert!(!state.pending);
        assert!(state.recent_authentication.is_none());
    }
    assert_eq!(clock.reads.load(Ordering::SeqCst), 20);
    assert_eq!(h.h.audit.query(&AuditFilter::default()).unwrap(), before);
    h.h.audit.fail.store(false, Ordering::SeqCst);
    h.h.random.fail.store(false, Ordering::SeqCst);
    // 同一有效OTP仍可提交；若GET预扣五次预算或消费过它，这里必然拒绝。
    runtime
        .step_up(&request(&owner, &issued, 1), otp(&OLD, NEXT))
        .unwrap();
}

#[test]
fn state_rechecks_csrf_clock_floor_idle_expiry_and_revocation() {
    let h = RecoveryHarness::new(60, 7200);
    let runtime = FactorRuntime::attach(h.h.manager.clone(), seed(OLD), Some(seed(NEW))).unwrap();
    let owner = identity();
    let first = h.h.manager.issue(&owner).unwrap();
    let second = h.h.manager.issue(&owner).unwrap();
    assert_eq!(
        runtime.state(&owner, &first.token, &second.csrf),
        Err(SessionError::Rejected)
    );
    h.h.clock.set(NEXT - 1);
    assert_eq!(
        runtime.state(&owner, &first.token, &first.csrf),
        Err(SessionError::ClockRollback)
    );
    h.h.clock.set(NEXT + 30_000);
    assert_eq!(
        runtime
            .state(&owner, &first.token, &first.csrf)
            .unwrap()
            .observed_at_ms,
        NEXT + 30_000
    );
    h.h.clock.set(NEXT + 90_000);
    assert_eq!(
        runtime.state(&owner, &first.token, &first.csrf),
        Err(SessionError::Rejected)
    );
    let fresh = h.h.manager.issue(&owner).unwrap();
    h.h.manager
        .revoke_current(&owner, &fresh.token, &fresh.csrf)
        .unwrap();
    assert_eq!(
        runtime.state(&owner, &fresh.token, &fresh.csrf),
        Err(SessionError::Rejected)
    );
}
