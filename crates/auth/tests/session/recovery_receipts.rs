//! 畸形当前回执不得落成坏库；迟到旧换发调用不能交付已被新批次替代的码。

#[path = "recovery_receipt_support.rs"]
mod receipt_support;

use super::*;
use receipt_support::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use studiolo_audit_ledger::{AuditPhase, AuditRecord};

#[test]
fn malformed_payload_wrong_event_id_and_zero_offset_never_become_current_acks() {
    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
        for fault in [
            Fault::Payload(phase),
            Fault::EventId(phase),
            Fault::ZeroOffset(phase),
        ] {
            let h = RecoveryHarness::new(3600, 7200);
            let (issued, codes) = h.ready();
            let (manager, recovery) = with_fault(&h, fault);
            let prepared = PreparedTotpFactor::prepare(manager.clone(), seed(NEW)).unwrap();
            h.h.clock.set(CHANGE);
            assert_eq!(
                recovery
                    .replace_factor(
                        &request(&identity(), &issued, 1),
                        copy_code(&codes, 0),
                        &prepared,
                        otp(&NEW, CHANGE),
                    )
                    .err()
                    .unwrap(),
                SessionError::AuditUnavailable
            );
            let acknowledged = i64::from(phase == AuditPhase::Outcome);
            assert_eq!(
                scalar(
                    &manager,
                    "SELECT count(*) FROM recovery_pending WHERE intent_receipt IS NOT NULL"
                ),
                acknowledged
            );
            assert_eq!(
                scalar(
                    &manager,
                    "SELECT count(*) FROM recovery_codes WHERE intent_receipt IS NOT NULL"
                ),
                acknowledged
            );
            assert_eq!(
                scalar(
                    &manager,
                    "SELECT count(*) FROM recovery_pending WHERE outcome_receipt IS NOT NULL"
                ),
                0
            );
            assert_eq!(
                scalar(
                    &manager,
                    "SELECT count(*) FROM recovery_codes WHERE outcome_receipt IS NOT NULL"
                ),
                0
            );
            assert_repaired_flush_still_cannot_switch(&h, &issued);
        }
    }
}

#[test]
fn self_consistent_outcome_at_lower_offset_is_rejected_before_ack_and_remains_repairable() {
    let h = RecoveryHarness::new(3600, 7200);
    let (issued, codes) = h.ready();
    let (manager, recovery) = with_fault(&h, Fault::LowOutcome);
    let prepared = PreparedTotpFactor::prepare(manager.clone(), seed(NEW)).unwrap();
    h.h.clock.set(CHANGE);
    assert_eq!(
        recovery
            .replace_factor(
                &request(&identity(), &issued, 1),
                copy_code(&codes, 0),
                &prepared,
                otp(&NEW, CHANGE),
            )
            .err()
            .unwrap(),
        SessionError::AuditUnavailable
    );
    let encoded: String = manager
        .database
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT intent_receipt FROM recovery_pending", [], |row| {
            row.get(0)
        })
        .unwrap();
    let intent: AuditRecord = serde_json::from_str(&encoded).unwrap();
    assert!(intent.offset() > 1);
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM recovery_pending WHERE outcome_receipt IS NOT NULL"
        ),
        0
    );
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM recovery_codes WHERE outcome_receipt IS NOT NULL"
        ),
        0
    );
    assert_repaired_flush_still_cannot_switch(&h, &issued);
}

#[test]
fn late_original_rotate_cannot_return_retired_codes_after_a_new_batch_has_activated() {
    const WAIT: Duration = Duration::from_secs(10);
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (issued, _) = h.ready();
    let issued = Arc::new(issued);
    let delayed_generation = scalar(&h.h.manager, "SELECT next_operation FROM recovery_control");
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let resume_rx = Mutex::new(resume_rx);
    let first = AtomicBool::new(true);
    *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        let generation = event
            .payload()
            .as_value()
            .get("generation")
            .and_then(serde_json::Value::as_i64);
        if event.action() == "recovery.codes_rotated"
            && event.phase() == AuditPhase::Outcome
            && generation == Some(delayed_generation)
            && first.swap(false, Ordering::SeqCst)
        {
            entered_tx.send(()).unwrap();
            resume_rx.lock().unwrap().recv_timeout(WAIT).unwrap();
        }
    }));
    let (_, delayed) = h.reopen();
    let moving = issued.clone();
    let worker = thread::spawn(move || delayed.rotate_codes(&request(&identity(), &moving, 1)));
    entered_rx.recv_timeout(WAIT).unwrap();
    assert!(h.recovery.flush_audit().unwrap() <= 2);
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        1
    );
    h.h.clock.set(NEXT + 300_000);
    h.recovery.flush_audit().unwrap();
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        0
    );
    let req = request(&owner, &issued, 1);
    h.totp.step_up(&req, otp(&OLD, NEXT + 300_000)).unwrap();
    let codes = h.recovery.rotate_codes(&req).unwrap();
    let current_generation = scalar(&h.h.manager, "SELECT min(generation) FROM recovery_codes");
    assert!(current_generation > delayed_generation);
    resume_tx.send(()).unwrap();
    assert!(worker.join().unwrap().is_err());
    *h.h.audit.hook.lock().unwrap() = None;
    assert_eq!(
        scalar(&h.h.manager, "SELECT min(generation) FROM recovery_codes"),
        current_generation
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_codes"),
        10
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        0
    );
    let now = NEXT + 330_000;
    h.h.clock.set(now);
    h.recovery
        .replace_factor(&req, copy_code(&codes, 0), &h.prepared(), otp(&NEW, now))
        .unwrap();
}
