//! 跨库故障允许永久烧码；补记、重开均不能代替原调用切换因子。

use super::*;
use std::sync::{Arc, atomic::Ordering};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase};

#[test]
fn failed_replacement_audit_burns_both_inputs_and_flush_never_switches_factor() {
    for fault in 0..3 {
        let h = RecoveryHarness::new(3600, 7200);
        let owner = identity();
        let (issued, codes) = h.ready();
        let prepared = h.prepared();
        h.h.clock.set(CHANGE);
        let weak = Arc::downgrade(&h.h.audit);
        *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
            if event.action() != "totp.recovered" {
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
        assert_eq!(
            scalar(&h.h.manager, "SELECT used_mask FROM recovery_control"),
            1
        );
        assert_eq!(
            scalar(&h.h.manager, "SELECT high_step FROM recovery_control"),
            CHANGE / 30_000
        );
        assert_eq!(
            scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
            1
        );
        assert_eq!(
            h.h.audit
                .query(&AuditFilter::default().with_action("totp.recovered"))
                .unwrap()
                .len(),
            fault
        );
        *h.h.audit.hook.lock().unwrap() = None;
        h.h.audit.fail_intent.store(false, Ordering::SeqCst);
        h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
        h.h.audit.after_append.store(false, Ordering::SeqCst);
        let (manager, recovery) = h.reopen();
        assert!(recovery.flush_audit().unwrap() <= 2);
        manager.check(&owner, &issued.token).unwrap();
        assert!(Totp::attach(manager.clone(), seed(OLD)).is_ok());
        assert_eq!(
            Totp::attach(manager.clone(), seed(NEW)).err().unwrap(),
            SessionError::Configuration
        );
        assert_eq!(scalar(&manager, "SELECT count(*) FROM recovery_pending"), 1);
        h.h.clock.set(CHANGE + 300_000);
        recovery.flush_audit().unwrap();
        assert_eq!(scalar(&manager, "SELECT count(*) FROM recovery_pending"), 0);
        let retry = PreparedTotpFactor::prepare(manager.clone(), seed(NEW)).unwrap();
        assert_eq!(
            recovery
                .replace_factor(
                    &request(&owner, &issued, 1),
                    copy_code(&codes, 0),
                    &retry,
                    otp(&NEW, CHANGE + 300_000)
                )
                .err()
                .unwrap(),
            SessionError::Rejected
        );
        recovery
            .replace_factor(
                &request(&owner, &issued, 1),
                copy_code(&codes, 1),
                &retry,
                otp(&NEW, CHANGE + 300_000),
            )
            .unwrap();
    }
}

#[test]
fn successful_ledger_append_with_failed_auth_ack_preserves_consumption_not_authority() {
    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
        let h = RecoveryHarness::new(3600, 7200);
        let owner = identity();
        let (issued, codes) = h.ready();
        let prepared = h.prepared();
        h.h.clock.set(CHANGE);
        let weak = Arc::downgrade(&h.h.manager);
        *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
            if event.action() == "totp.recovered" && event.phase() == phase {
                execute(&weak.upgrade().unwrap(), "PRAGMA query_only=ON");
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
            SessionError::StorageUnavailable
        );
        assert_eq!(
            scalar(&h.h.manager, "SELECT used_mask FROM recovery_control"),
            1
        );
        assert_eq!(
            scalar(
                &h.h.manager,
                "SELECT count(*) FROM recovery_pending WHERE outcome_receipt IS NOT NULL"
            ),
            0
        );
        assert_eq!(
            h.h.audit
                .query(&AuditFilter::default().with_action("totp.recovered"))
                .unwrap()
                .len(),
            if phase == AuditPhase::Intent { 1 } else { 2 }
        );
        *h.h.audit.hook.lock().unwrap() = None;
        execute(&h.h.manager, "PRAGMA query_only=OFF");
        assert!(h.recovery.flush_audit().unwrap() <= 2);
        h.h.manager.check(&owner, &issued.token).unwrap();
        assert!(Totp::attach(h.h.manager.clone(), seed(OLD)).is_ok());
        assert_eq!(
            scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
            1
        );
    }
}

#[test]
fn failed_rotation_never_returns_codes_or_retires_the_confirmed_batch() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (issued, codes) = h.ready();
    let current = scalar(&h.h.manager, "SELECT min(generation) FROM recovery_codes");
    let weak = Arc::downgrade(&h.h.audit);
    *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "recovery.codes_rotated" && event.phase() == AuditPhase::Outcome {
            // 只使新批次失败，不在已确认批次的回执重放上注入。
            let generation = event
                .payload()
                .as_value()
                .get("generation")
                .and_then(serde_json::Value::as_i64)
                .unwrap();
            if generation != current {
                weak.upgrade()
                    .unwrap()
                    .fail_outcome
                    .store(true, Ordering::SeqCst);
            }
        }
    }));
    assert_eq!(
        h.recovery
            .rotate_codes(&request(&owner, &issued, 1))
            .err()
            .unwrap(),
        SessionError::AuditUnavailable
    );
    assert!(scalar(&h.h.manager, "SELECT count(*) FROM recovery_codes") <= 20);
    *h.h.audit.hook.lock().unwrap() = None;
    h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
    h.h.clock.set(NEXT + 300_000);
    h.recovery.flush_audit().unwrap();
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_codes"),
        10
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT min(generation) FROM recovery_codes"),
        current
    );
    h.recovery
        .replace_factor(
            &request(&owner, &issued, 1),
            copy_code(&codes, 0),
            &h.prepared(),
            otp(&NEW, NEXT + 300_000),
        )
        .unwrap();
}
