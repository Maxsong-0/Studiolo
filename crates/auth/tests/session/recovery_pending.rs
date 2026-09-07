//! TOTP 与恢复共用单所有者互斥边界；安全吊销不被审计故障阻塞。

use super::*;
use std::sync::{Arc, atomic::Ordering};
use studiolo_audit_ledger::AuditPhase;

#[test]
fn recovery_pending_blocks_totp_and_rotation_but_does_not_block_legitimate_revocation() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (issued, codes) = h.ready();
    let controller = h.h.manager.issue(&owner).unwrap();
    let prepared = h.prepared();
    h.h.clock.set(CHANGE);
    let weak = Arc::downgrade(&h.h.audit);
    *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "totp.recovered" && event.phase() == AuditPhase::Intent {
            weak.upgrade()
                .unwrap()
                .fail_intent
                .store(true, Ordering::SeqCst);
        }
    }));
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
        SessionError::AuditUnavailable
    );
    *h.h.audit.hook.lock().unwrap() = None;
    h.h.audit.fail_intent.store(false, Ordering::SeqCst);
    h.h.clock.set(CHANGE + 30_000);
    assert_eq!(
        h.totp
            .step_up(&request(&owner, &issued, 1), otp(&OLD, CHANGE + 30_000))
            .unwrap_err(),
        SessionError::OperationPending
    );
    assert_eq!(
        h.recovery
            .rotate_codes(&request(&owner, &issued, 1))
            .err()
            .unwrap(),
        SessionError::OperationPending
    );
    h.h.audit.fail.store(true, Ordering::SeqCst);
    h.h.manager
        .revoke(
            &owner,
            &controller.token,
            &controller.csrf,
            &issued.session.id,
        )
        .unwrap();
    assert_eq!(
        h.h.manager.check(&owner, &issued.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        1
    );
    h.h.audit.fail.store(false, Ordering::SeqCst);
    h.recovery.flush_audit().unwrap();
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        0
    );
    // 候选消费水位仍保留，清理不能让旧因子在同一步重新消费。
    assert_eq!(
        scalar(&h.h.manager, "SELECT high_step FROM recovery_control"),
        CHANGE / 30_000
    );
    assert_eq!(
        h.totp
            .step_up(&request(&owner, &controller, 1), otp(&OLD, CHANGE))
            .unwrap_err(),
        SessionError::Rejected
    );
    h.totp
        .step_up(&request(&owner, &controller, 1), otp(&OLD, CHANGE + 30_000))
        .unwrap();
}

#[test]
fn totp_pending_blocks_recovery_without_burning_code_or_creating_second_pending() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (issued, codes) = h.ready();
    let prepared = h.prepared();
    h.h.clock.set(NEXT + 30_000);
    let weak = Arc::downgrade(&h.h.audit);
    *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "totp.step_up" && event.phase() == AuditPhase::Intent {
            // 已确认旧 proof 在 NEXT，故只注入后续这一笔。
            let verified = event
                .payload()
                .as_value()
                .get("verified_at_ms")
                .and_then(serde_json::Value::as_i64)
                .unwrap();
            if verified != NEXT {
                weak.upgrade()
                    .unwrap()
                    .fail_intent
                    .store(true, Ordering::SeqCst);
            }
        }
    }));
    assert_eq!(
        h.totp
            .step_up(&request(&owner, &issued, 1), otp(&OLD, NEXT + 30_000))
            .unwrap_err(),
        SessionError::AuditUnavailable
    );
    *h.h.audit.hook.lock().unwrap() = None;
    h.h.audit.fail_intent.store(false, Ordering::SeqCst);
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
        SessionError::OperationPending
    );
    assert_eq!(
        h.recovery
            .rotate_codes(&request(&owner, &issued, 1))
            .err()
            .unwrap(),
        SessionError::OperationPending
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT used_mask FROM recovery_control"),
        0
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        0
    );
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 1);
}
