//! 测试私有数据库损坏和秘密边界；这些 SQL 注入只作用于临时虚构夹具。

use super::*;
use std::sync::Arc;
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase};

#[test]
fn wrong_preconfigured_reference_or_seed_cannot_replace_an_enrolled_factor() {
    let h = TotpHarness::enrolled(3600, 7200);
    for different in [
        TotpSeed::new("test-only-another-reference".into(), SEED).unwrap(),
        TotpSeed::new(SEED_REF.into(), [77; 20]).unwrap(),
    ] {
        assert_eq!(
            Totp::attach(h.h.manager.clone(), different).err().unwrap(),
            SessionError::Configuration
        );
    }
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_factor"), 1);
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 0);
    assert_eq!(
        text_rows(&h.h.manager, "SELECT seed_ref FROM totp_factor"),
        vec![SEED_REF.to_owned()]
    );
}

#[test]
fn factor_counter_receipt_and_oversized_metadata_corruption_fail_before_l3() {
    for mutation in [
        "UPDATE totp_factor SET last_step=last_step-1",
        "UPDATE totp_factor SET seed_ref=hex(zeroblob(2048))",
        "UPDATE totp_factor SET consumption_json='{}'",
        "UPDATE totp_grants SET operation_json='{}'",
        "UPDATE totp_grants SET intent_receipt='{}'",
        "UPDATE totp_grants SET outcome_receipt=hex(zeroblob(8192))",
        "UPDATE totp_grants SET activated_at=activated_at+900000",
        "UPDATE totp_control SET next_operation=1",
    ] {
        let h = TotpHarness::enrolled(3600, 7200);
        let owner = identity();
        let issued = h.h.manager.issue(&owner).unwrap();
        h.totp
            .step_up(&request(&owner, &issued, 1), code(NEXT))
            .unwrap();
        h.h.manager
            .database
            .connection
            .lock()
            .unwrap()
            .execute_batch(mutation)
            .unwrap();
        assert_eq!(
            h.totp
                .require_l3(
                    &request(&owner, &issued, 1),
                    SensitiveAction::TestCredential
                )
                .unwrap_err(),
            SessionError::CorruptStore,
            "{mutation}"
        );
        assert!(h.totp.flush_audit().is_err());
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 1);
    }
}

#[test]
fn budget_write_failure_does_not_consume_code_or_send_audit() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    let before = h.h.audit.query(&AuditFilter::default()).unwrap();
    let step = scalar(&h.h.manager, "SELECT last_step FROM totp_factor");
    h.h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute_batch("PRAGMA query_only=ON")
        .unwrap();
    assert_eq!(
        h.totp
            .step_up(&request(&owner, &issued, 1), code(NEXT))
            .unwrap_err(),
        SessionError::StorageUnavailable
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT last_step FROM totp_factor"),
        step
    );
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 0);
    assert_eq!(h.h.audit.query(&AuditFilter::default()).unwrap(), before);
    h.h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute_batch("PRAGMA query_only=OFF")
        .unwrap();
    h.totp
        .step_up(&request(&owner, &issued, 1), code(NEXT))
        .unwrap();
}

#[test]
fn final_auth_write_failure_after_audit_does_not_publish_on_later_flush() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    let weak = Arc::downgrade(&h.h.manager);
    *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "totp.step_up" && event.phase() == AuditPhase::Outcome {
            weak.upgrade()
                .unwrap()
                .database
                .connection
                .lock()
                .unwrap()
                .execute_batch("PRAGMA query_only=ON")
                .unwrap();
        }
    }));
    assert_eq!(
        h.totp
            .step_up(&request(&owner, &issued, 1), code(NEXT))
            .unwrap_err(),
        SessionError::StorageUnavailable
    );
    *h.h.audit.hook.lock().unwrap() = None;
    h.h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute_batch("PRAGMA query_only=OFF")
        .unwrap();
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 0);
    let (_, totp, _) = h.reopen();
    totp.flush_audit().unwrap();
    assert_eq!(
        totp.require_l3(&request(&owner, &issued, 1), SensitiveAction::ChangeBackup)
            .unwrap_err(),
        SessionError::ReauthenticationRequired
    );
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 1);
}
