//! 失败或过期的持久决定仅是显示事实；状态读取不补记、不清理、更不重放。

use super::*;
use std::sync::atomic::Ordering;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn pending_step_up_keeps_the_previous_grant_until_its_original_deadline() {
    let h = RecoveryHarness::new(3600, 7200);
    let runtime = FactorRuntime::attach(h.h.manager.clone(), seed(OLD), Some(seed(NEW))).unwrap();
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    let req = request(&owner, &issued, 1);
    let original = runtime.step_up(&req, otp(&OLD, NEXT)).unwrap();
    h.h.clock.set(CHANGE);
    h.h.audit.fail_outcome.store(true, Ordering::SeqCst);
    assert_eq!(
        runtime.step_up(&req, otp(&OLD, CHANGE)),
        Err(SessionError::AuditUnavailable)
    );
    let before = h.h.audit.query(&AuditFilter::default()).unwrap();
    let state = runtime.state(&owner, &issued.token, &issued.csrf).unwrap();
    assert!(state.pending);
    assert_eq!(state.recent_authentication, Some(original));
    h.h.clock.set(original.expires_at_ms);
    let state = runtime.state(&owner, &issued.token, &issued.csrf).unwrap();
    assert!(state.pending);
    assert!(state.recent_authentication.is_none());
    h.h.clock.set(CHANGE + 300_000);
    assert!(
        runtime
            .state(&owner, &issued.token, &issued.csrf)
            .unwrap()
            .pending
    );
    assert_eq!(h.h.audit.query(&AuditFilter::default()).unwrap(), before);
    h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
    h.totp.flush_audit().unwrap();
    let state = runtime.state(&owner, &issued.token, &issued.csrf).unwrap();
    assert!(!state.pending);
    assert!(state.recent_authentication.is_none());
    assert_eq!(
        runtime.rotate_codes(&req).err().unwrap(),
        SessionError::ReauthenticationRequired
    );
}

#[test]
fn recovery_pending_stays_visible_after_live_flush_and_consumption_never_reverses() {
    let h = RecoveryHarness::new(3600, 7200);
    let runtime = FactorRuntime::attach(h.h.manager.clone(), seed(OLD), Some(seed(NEW))).unwrap();
    let owner = identity();
    let (issued, codes) = h.ready();
    let req = request(&owner, &issued, 1);
    h.h.clock.set(CHANGE);
    h.h.audit.fail_outcome.store(true, Ordering::SeqCst);
    assert_eq!(
        runtime.recover(&req, copy_code(&codes, 0), otp(&NEW, CHANGE)),
        Err(SessionError::AuditUnavailable)
    );
    let before = h.h.audit.query(&AuditFilter::default()).unwrap();
    let state = runtime.state(&owner, &issued.token, &issued.csrf).unwrap();
    assert!(state.pending);
    assert!(state.recovery_candidate_available);
    assert_eq!(state.recent_authentication.unwrap().verified_at_ms, NEXT);
    assert_eq!(h.h.audit.query(&AuditFilter::default()).unwrap(), before);
    h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
    h.recovery.flush_audit().unwrap();
    assert!(
        runtime
            .state(&owner, &issued.token, &issued.csrf)
            .unwrap()
            .pending
    );
    h.h.clock.set(CHANGE + 300_000);
    let state = runtime.state(&owner, &issued.token, &issued.csrf).unwrap();
    assert!(state.pending);
    assert!(state.recent_authentication.is_none());
    h.recovery.flush_audit().unwrap();
    let state = runtime.state(&owner, &issued.token, &issued.csrf).unwrap();
    assert!(!state.pending);
    assert!(state.recovery_candidate_available);
    assert_eq!(
        runtime.recover(&req, copy_code(&codes, 0), otp(&NEW, CHANGE + 300_000)),
        Err(SessionError::Rejected)
    );
    runtime
        .recover(&req, copy_code(&codes, 1), otp(&NEW, CHANGE + 300_000))
        .unwrap();
    assert_eq!(
        runtime.state(&owner, &issued.token, &issued.csrf),
        Err(SessionError::Rejected)
    );
}
