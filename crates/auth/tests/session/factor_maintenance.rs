//! 运行维护覆盖所有 outbox，但不能让失败动作获得原调用的提交权限。

use super::*;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn rotation_maintenance_delivers_originals_without_returning_or_activating_codes() {
    let h = RecoveryHarness::new(3600, 7200);
    let runtime = attach(h.h.manager.clone());
    let owner = identity();
    let (issued, codes) = h.ready();
    let req = request(&owner, &issued, 1);
    let generation = scalar(&h.h.manager, "SELECT min(generation) FROM recovery_codes");
    h.h.audit.fail_outcome.store(true, Ordering::SeqCst);
    assert_eq!(
        runtime.rotate_codes(&req).err().unwrap(),
        SessionError::AuditUnavailable
    );
    assert_eq!(runtime.flush_audit(), Err(SessionError::AuditUnavailable));
    h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
    assert!(runtime.flush_audit().unwrap() <= 260);
    assert!(
        runtime
            .state(&owner, &issued.token, &issued.csrf)
            .unwrap()
            .pending
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT min(generation) FROM recovery_codes"),
        generation
    );
    assert_eq!(runtime.flush_audit().unwrap(), 0);
    h.h.clock.set(NEXT + 300_000);
    runtime.flush_audit().unwrap();
    assert!(
        !runtime
            .state(&owner, &issued.token, &issued.csrf)
            .unwrap()
            .pending
    );
    assert_eq!(
        runtime.rotate_codes(&req).err().unwrap(),
        SessionError::ReauthenticationRequired
    );
    // 失败的新批次没有替代旧恢复码；原码仍可在新调用中恢复至预配置候选。
    runtime
        .recover(&req, copy_code(&codes, 0), otp(&NEW, NEXT + 300_000))
        .unwrap();
}

#[test]
fn maintenance_flushes_totp_and_denials_without_reinstating_expired_l3() {
    let h = RecoveryHarness::new(3600, 7200);
    let runtime = attach(h.h.manager.clone());
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    let req = request(&owner, &issued, 1);
    h.h.audit.fail_outcome.store(true, Ordering::SeqCst);
    assert_eq!(
        runtime.step_up(&req, otp(&OLD, NEXT)),
        Err(SessionError::AuditUnavailable)
    );
    h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
    h.h.clock.set(NEXT + 300_000);
    assert!(runtime.flush_audit().unwrap() <= 260);
    let state = runtime.state(&owner, &issued.token, &issued.csrf).unwrap();
    assert!(!state.pending);
    assert!(state.recent_authentication.is_none());
    for action in ["totp.step_up", "session.denied"] {
        assert!(
            !h.h.audit
                .query(&AuditFilter::default().with_action(action))
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(runtime.flush_audit().unwrap(), 0);
}
