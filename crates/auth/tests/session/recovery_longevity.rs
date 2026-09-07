//! 真实 64 轮业务操作而非扩大预算的 SQL 夹具；Auth 当前证明与行数保持有界。

use super::*;
use std::{collections::HashSet, sync::atomic::Ordering};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn sixty_four_real_rotations_keep_only_one_current_batch_and_fixed_control_state() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    let req = request(&owner, &issued, 1);
    h.totp.step_up(&req, otp(&OLD, NEXT)).unwrap();
    let mut seen = HashSet::new();
    let mut first = None;
    let mut latest = None;
    let mut generation = 0;
    for index in 0..64 {
        let codes = h.recovery.rotate_codes(&req).unwrap();
        if index == 0 {
            first = Some(copy_code(&codes, 0));
        }
        for code in codes.codes() {
            assert!(seen.insert(code.expose_secret().to_owned()));
        }
        let current = scalar(&h.h.manager, "SELECT min(generation) FROM recovery_codes");
        assert!(current > generation);
        generation = current;
        assert_eq!(
            scalar(&h.h.manager, "SELECT count(*) FROM recovery_codes"),
            10
        );
        assert_eq!(
            scalar(&h.h.manager, "SELECT count(*) FROM recovery_control"),
            1
        );
        assert_eq!(
            scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
            0
        );
        assert_eq!(
            scalar(&h.h.manager, "SELECT count(*) FROM recovery_rates"),
            0
        );
        assert!(
            scalar(
                &h.h.manager,
                "SELECT octet_length(current_json) FROM recovery_control"
            ) <= 65_536
        );
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 1);
        latest = Some(codes);
    }
    assert_eq!(seen.len(), 640);
    assert_eq!(
        h.h.audit
            .query(&AuditFilter::default().with_action("recovery.codes_rotated"))
            .unwrap()
            .len(),
        128
    );
    let (manager, recovery) = h.reopen();
    let prepared = PreparedTotpFactor::prepare(manager.clone(), seed(NEW)).unwrap();
    h.h.clock.set(CHANGE);
    assert_eq!(
        recovery
            .replace_factor(&req, first.unwrap(), &prepared, otp(&NEW, CHANGE))
            .err()
            .unwrap(),
        SessionError::Rejected
    );
    recovery
        .replace_factor(
            &req,
            copy_code(&latest.unwrap(), 0),
            &prepared,
            otp(&NEW, CHANGE),
        )
        .unwrap();
    assert_eq!(scalar(&manager, "SELECT count(*) FROM recovery_codes"), 0);
    assert_eq!(scalar(&manager, "SELECT count(*) FROM recovery_control"), 1);
    assert!(
        scalar(
            &manager,
            "SELECT octet_length(change_json) FROM recovery_control"
        ) <= 65_536
    );
}

#[test]
fn random_failure_cannot_retire_existing_codes_or_leave_a_staged_batch() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (issued, codes) = h.ready();
    let generation = scalar(&h.h.manager, "SELECT min(generation) FROM recovery_codes");
    h.h.random.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        h.recovery
            .rotate_codes(&request(&owner, &issued, 1))
            .err()
            .unwrap(),
        SessionError::RandomUnavailable
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        0
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_codes"),
        10
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT min(generation) FROM recovery_codes"),
        generation
    );
    h.h.random.fail.store(false, Ordering::SeqCst);
    h.h.clock.set(CHANGE);
    h.recovery
        .replace_factor(
            &request(&owner, &issued, 1),
            copy_code(&codes, 0),
            &h.prepared(),
            otp(&NEW, CHANGE),
        )
        .unwrap();
}
