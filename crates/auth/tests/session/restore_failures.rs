//! 审计故障不复活权限；所有撤权前拒绝保持 Auth 原样。
use super::*;

#[test]
fn wrong_ledger_and_invalid_ids_leave_auth_unchanged() {
    let h = RecoveryHarness::new(3600, 7200);
    h.ready();
    let before = whole(&h.h.manager);
    for id in ["", "has space", "a/b", "é", &"x".repeat(56)] {
        assert_eq!(
            h.h.manager.invalidate_restored_credentials(id).unwrap_err(),
            SessionError::Configuration
        );
    }
    let records = h.h.audit.query(&AuditFilter::default()).unwrap();
    let ledger = Arc::new(MemoryAuditLedger::default());
    ledger
        .append(records.last().unwrap().event().clone())
        .unwrap();
    let other = SessionManager::open(
        &h.h.root,
        config(3600, 7200),
        ledger,
        h.h.clock.clone(),
        h.h.random.clone(),
    )
    .unwrap();
    assert_eq!(
        other
            .invalidate_restored_credentials("other-ledger")
            .unwrap_err(),
        SessionError::AuditUnavailable
    );
    assert_eq!(whole(&h.h.manager), before);
}

#[test]
fn append_failure_before_or_after_commit_keeps_revocation_and_retries_original_pair() {
    for after in [false, true] {
        let h = RecoveryHarness::new(3600, 7200);
        let (old, _) = h.ready();
        let weak = Arc::downgrade(&h.h.audit);
        *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
            if event.payload().as_value()["scope"] == "restore" {
                let audit = weak.upgrade().unwrap();
                if after {
                    audit.after_append.store(true, Ordering::SeqCst);
                } else {
                    audit.fail_outcome.store(true, Ordering::SeqCst);
                }
            }
        }));
        let mut first =
            h.h.manager
                .invalidate_restored_credentials("fault")
                .unwrap();
        assert!(!first.audit_confirmed);
        assert_eq!(first.epoch, 2);
        assert_eq!(
            scalar(&h.h.manager, "SELECT count(*) FROM recovery_codes"),
            0
        );
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 0);
        *h.h.audit.hook.lock().unwrap() = None;
        h.h.audit.after_append.store(false, Ordering::SeqCst);
        h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
        assert_eq!(
            h.h.manager.check(&identity(), &old.token).unwrap_err(),
            SessionError::Rejected
        );
        let fresh = h.h.manager.issue(&identity()).unwrap();
        first.audit_confirmed = true;
        assert_eq!(
            h.h.manager
                .invalidate_restored_credentials("fault")
                .unwrap(),
            first
        );
        h.h.manager.check(&identity(), &fresh.token).unwrap();
        let events =
            h.h.audit
                .query(&AuditFilter::default().with_action("session.revoked"))
                .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|r| r.event().payload().as_value()["scope"] == "restore")
                .count(),
            2
        );
    }
}

#[test]
fn original_proof_write_failure_or_changed_snapshot_does_not_apply_restore() {
    for mutate in [false, true] {
        let h = RecoveryHarness::new(3600, 7200);
        h.ready();
        let before = whole(&h.h.manager);
        let weak = Arc::downgrade(&h.h.manager);
        let once = Arc::new(std::sync::atomic::AtomicBool::new(false));
        *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |_| {
            if !once.swap(true, Ordering::SeqCst) {
                execute(
                    &weak.upgrade().unwrap(),
                    if mutate {
                        "UPDATE auth_meta SET clock_floor=clock_floor+1"
                    } else {
                        "PRAGMA query_only=ON"
                    },
                );
            }
        }));
        assert_eq!(
            h.h.manager
                .invalidate_restored_credentials("failure")
                .unwrap_err(),
            if mutate {
                SessionError::OperationPending
            } else {
                SessionError::StorageUnavailable
            }
        );
        *h.h.audit.hook.lock().unwrap() = None;
        if mutate {
            execute(
                &h.h.manager,
                "UPDATE auth_meta SET clock_floor=clock_floor-1",
            );
        } else {
            execute(&h.h.manager, "PRAGMA query_only=OFF");
        }
        assert_eq!(whole(&h.h.manager), before);
    }
}

#[test]
fn unfinished_totp_recovery_and_retention_are_not_discarded() {
    for kind in ["totp", "recovery", "retention"] {
        let h = RecoveryHarness::new(3600, 7200);
        let (issued, codes) = h.ready();
        let prepared = h.prepared();
        h.h.clock.set(CHANGE);
        if kind == "retention" {
            h.h.manager
                .revoke_all(&identity(), &issued.token, &issued.csrf)
                .unwrap();
        }
        let weak = Arc::downgrade(&h.h.audit);
        *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
            let fail = match kind {
                "totp" => {
                    event.action() == "totp.step_up"
                        && event.payload().as_value()["verified_at_ms"] != NEXT
                }
                "recovery" => event.action() == "totp.recovered",
                _ => event.action() == "session.compacted",
            };
            if fail {
                weak.upgrade().unwrap().fail.store(true, Ordering::SeqCst);
            }
        }));
        match kind {
            "totp" => {
                assert!(
                    h.totp
                        .step_up(&request(&identity(), &issued, 1), otp(&OLD, CHANGE))
                        .is_err()
                );
            }
            "recovery" => {
                assert!(
                    h.recovery
                        .replace_factor(
                            &request(&identity(), &issued, 1),
                            copy_code(&codes, 0),
                            &prepared,
                            otp(&NEW, CHANGE)
                        )
                        .is_err()
                );
            }
            _ => {
                assert!(h.h.manager.maintain_retention().is_err());
            }
        }
        *h.h.audit.hook.lock().unwrap() = None;
        h.h.audit.fail.store(false, Ordering::SeqCst);
        let before = whole(&h.h.manager);
        assert_eq!(
            h.h.manager
                .invalidate_restored_credentials("pending")
                .unwrap_err(),
            SessionError::OperationPending
        );
        assert_eq!(whole(&h.h.manager), before);
    }
}

#[test]
fn retry_without_restore_receipts_cannot_change_the_original_factor_ledger() {
    let h = RecoveryHarness::new(3600, 7200);
    h.ready();
    let weak = Arc::downgrade(&h.h.audit);
    *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.payload().as_value()["scope"] == "restore" {
            weak.upgrade().unwrap().fail.store(true, Ordering::SeqCst);
        }
    }));
    assert!(
        !h.h.manager
            .invalidate_restored_credentials("no-receipts")
            .unwrap()
            .audit_confirmed
    );
    *h.h.audit.hook.lock().unwrap() = None;
    h.h.audit.fail.store(false, Ordering::SeqCst);
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM audit_outbox WHERE event_id LIKE '%:restore:%' AND receipt_json IS NOT NULL"
        ),
        0
    );
    let before = whole(&h.h.manager);
    let other = Arc::new(MemoryAuditLedger::default());
    let records = h.h.audit.query(&AuditFilter::default()).unwrap();
    other
        .append(records.last().unwrap().event().clone())
        .unwrap();
    let manager = SessionManager::open(
        &h.h.root,
        config(3600, 7200),
        other.clone(),
        h.h.clock.clone(),
        h.h.random.clone(),
    )
    .unwrap();
    let report = manager
        .invalidate_restored_credentials("no-receipts")
        .unwrap();
    assert_eq!(report.epoch, 2);
    assert!(!report.audit_confirmed);
    assert_eq!(whole(&h.h.manager), before);
    assert!(
        other
            .query(&AuditFilter::default().with_action("session.revoked"))
            .unwrap()
            .is_empty()
    );
    assert!(
        h.h.manager
            .invalidate_restored_credentials("no-receipts")
            .unwrap()
            .audit_confirmed
    );
}
