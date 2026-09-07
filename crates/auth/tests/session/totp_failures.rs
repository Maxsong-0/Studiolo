//! 消费与审计跨库失败：可以烧代码，绝不能靠补记发布 L3。

use super::*;
use std::sync::{Arc, atomic::Ordering};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase};

#[test]
fn audit_intent_outcome_and_after_append_failure_burn_code_but_flush_never_elevates() {
    for fault in 0..3 {
        let h = TotpHarness::enrolled(3600, 7200);
        let owner = identity();
        let issued = h.h.manager.issue(&owner).unwrap();
        let weak = Arc::downgrade(&h.h.audit);
        *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
            // 已登记证明的重确认必须先成功，故障只落到本次 step-up 的指定阶段。
            if event.action() != "totp.step_up" {
                return;
            }
            let audit = weak.upgrade().unwrap();
            if fault == 0 && event.phase() == AuditPhase::Intent {
                audit.fail_intent.store(true, Ordering::SeqCst);
            } else if event.phase() == AuditPhase::Outcome {
                audit.fail_outcome.store(fault == 1, Ordering::SeqCst);
                audit.after_append.store(fault == 2, Ordering::SeqCst);
            }
        }));
        assert_eq!(
            h.totp
                .step_up(&request(&owner, &issued, 1), code(NEXT))
                .unwrap_err(),
            SessionError::AuditUnavailable
        );
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 1);
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 0);
        assert_eq!(
            h.h.audit
                .query(&AuditFilter::default().with_action("totp.step_up"))
                .unwrap()
                .len(),
            fault
        );
        assert_eq!(
            scalar(
                &h.h.manager,
                "SELECT count(*) FROM totp_pending WHERE intent_receipt IS NOT NULL"
            ),
            i64::from(fault != 0)
        );
        assert_eq!(
            scalar(
                &h.h.manager,
                "SELECT count(*) FROM totp_pending WHERE outcome_receipt IS NOT NULL"
            ),
            0
        );
        *h.h.audit.hook.lock().unwrap() = None;
        h.h.audit.fail_intent.store(false, Ordering::SeqCst);
        h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
        h.h.audit.after_append.store(false, Ordering::SeqCst);
        let (_, totp, _) = h.reopen();
        assert!(totp.flush_audit().unwrap() <= 2);
        assert_eq!(
            totp.require_l3(&request(&owner, &issued, 1), SensitiveAction::RestoreVault)
                .unwrap_err(),
            SessionError::ReauthenticationRequired
        );
        assert_eq!(
            totp.step_up(&request(&owner, &issued, 1), code(NEXT))
                .unwrap_err(),
            SessionError::OperationPending
        );
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 1);
        h.h.clock.set(NEXT + 300_000);
        assert!(totp.flush_audit().unwrap() <= 2);
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 0);
        totp.step_up(&request(&owner, &issued, 1), code(NEXT + 300_000))
            .unwrap();
        assert_eq!(
            h.h.audit
                .query(&AuditFilter::default().with_action("totp.step_up"))
                .unwrap()
                .len(),
            4
        );
    }
}

#[test]
fn failed_enrollment_keeps_binding_and_consumed_step_until_audited_expiry_cleanup() {
    let h = TotpHarness::new(3600, 7200);
    h.h.audit.fail_outcome.store(true, Ordering::SeqCst);
    assert_eq!(
        h.enrollment.enroll(code(START)).unwrap_err(),
        SessionError::AuditUnavailable
    );
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_factor"), 1);
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 1);
    h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
    let (manager, totp, enrollment) = h.reopen();
    assert!(totp.flush_audit().unwrap() <= 2);
    let owner = identity();
    let issued = manager.issue(&owner).unwrap();
    assert!(
        totp.require_l3(&request(&owner, &issued, 1), SensitiveAction::ChangeVault)
            .is_err()
    );
    assert_eq!(
        enrollment.enroll(code(START)).unwrap_err(),
        SessionError::OperationPending
    );
    let different = TotpSeed::new("test-only-other".into(), [7; 20]).unwrap();
    assert_eq!(
        Totp::attach(manager.clone(), different).err().unwrap(),
        SessionError::Configuration
    );
    h.h.clock.set(START + 300_000);
    totp.flush_audit().unwrap();
    enrollment.enroll(code(START + 300_000)).unwrap();
    assert_eq!(scalar(&manager, "SELECT count(*) FROM totp_pending"), 0);
    assert_eq!(scalar(&manager, "SELECT count(*) FROM totp_grants"), 0);
}

#[test]
fn audit_delay_cannot_extend_the_consumption_window_or_publish_after_deadline() {
    for delayed in [NEXT + 299_999, NEXT + 300_000] {
        let h = TotpHarness::enrolled(3600, 7200);
        let owner = identity();
        let issued = h.h.manager.issue(&owner).unwrap();
        let clock = h.h.clock.clone();
        *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
            if event.action() == "totp.step_up" && event.phase() == AuditPhase::Outcome {
                clock.set(delayed);
            }
        }));
        let result = h.totp.step_up(&request(&owner, &issued, 1), code(NEXT));
        if delayed < NEXT + 300_000 {
            let proof = result.unwrap();
            assert_eq!(proof.verified_at_ms, NEXT);
            assert_eq!(proof.expires_at_ms, NEXT + 300_000);
        } else {
            assert!(result.is_err());
            assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 0);
        }
        *h.h.audit.hook.lock().unwrap() = None;
        h.h.clock.set(NEXT + 300_000);
        assert_eq!(
            h.totp
                .require_l3(&request(&owner, &issued, 1), SensitiveAction::ReleaseFreeze)
                .unwrap_err(),
            SessionError::ReauthenticationRequired
        );
    }
}
