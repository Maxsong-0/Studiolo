//! 单字段破坏与超长值必须失败关闭；这不声称抵抗管理员整库一致回滚。

use super::*;
use std::sync::{Arc, atomic::Ordering};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, MemoryAuditLedger};

#[test]
fn changed_batch_digest_bitmap_receipt_or_oversized_proof_is_rejected_on_read_and_reopen() {
    for sql in [
        "UPDATE recovery_codes SET digest=lower(hex(zeroblob(32))) WHERE slot=0",
        "UPDATE recovery_control SET used_mask=1",
        "UPDATE recovery_control SET current_json=json_set(current_json,'$.activated_at',0)",
        "UPDATE recovery_control SET current_json=json_set(current_json,'$.outcome_receipt','{}')",
        "UPDATE recovery_control SET current_json=replace(hex(zeroblob(40000)),'0','x')",
        "UPDATE recovery_codes SET digest=replace(hex(zeroblob(40000)),'0','x') WHERE slot=0",
        "DELETE FROM recovery_codes WHERE slot=9",
    ] {
        let h = RecoveryHarness::new(3600, 7200);
        let (issued, _) = h.ready();
        execute(&h.h.manager, sql);
        assert_eq!(
            h.h.manager.check(&identity(), &issued.token).unwrap_err(),
            SessionError::CorruptStore
        );
        assert_eq!(
            h.recovery.flush_audit().unwrap_err(),
            SessionError::CorruptStore
        );
        assert_eq!(
            SessionManager::open(
                &h.h.root,
                config(3600, 7200),
                h.h.audit.clone(),
                h.h.clock.clone(),
                h.h.random.clone()
            )
            .err()
            .unwrap(),
            SessionError::CorruptStore
        );
    }
}

#[test]
fn pending_consumption_anchor_and_high_step_cannot_be_individually_rolled_back() {
    for sql in [
        "UPDATE recovery_control SET high_step=high_step-1",
        "UPDATE recovery_control SET used_mask=0",
        "UPDATE recovery_control SET consumption_json=NULL",
        "UPDATE recovery_pending SET operation_json=replace(hex(zeroblob(40000)),'0','x')",
        "UPDATE recovery_pending SET intent_receipt=replace(hex(zeroblob(40000)),'0','x')",
        "UPDATE recovery_codes SET consumption_json=NULL,intent_receipt=NULL,outcome_receipt=NULL WHERE slot=0",
    ] {
        let h = RecoveryHarness::new(3600, 7200);
        let (issued, codes) = h.ready();
        let prepared = h.prepared();
        h.h.clock.set(CHANGE);
        let weak = Arc::downgrade(&h.h.audit);
        *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
            if event.action() == "totp.recovered" && event.phase() == AuditPhase::Outcome {
                weak.upgrade()
                    .unwrap()
                    .fail_outcome
                    .store(true, Ordering::SeqCst);
            }
        }));
        assert_eq!(
            h.recovery
                .replace_factor(
                    &request(&identity(), &issued, 1),
                    copy_code(&codes, 0),
                    &prepared,
                    otp(&NEW, CHANGE)
                )
                .err()
                .unwrap(),
            SessionError::AuditUnavailable
        );
        *h.h.audit.hook.lock().unwrap() = None;
        h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
        execute(&h.h.manager, sql);
        assert_eq!(
            h.recovery.flush_audit().unwrap_err(),
            SessionError::CorruptStore
        );
        assert_eq!(
            h.h.manager.check(&identity(), &issued.token).unwrap_err(),
            SessionError::CorruptStore
        );
    }
}

#[test]
fn a_different_valid_ledger_receipt_cannot_retire_the_original_batch() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (issued, _) = h.ready();
    let records = h.h.audit.query(&AuditFilter::default()).unwrap();
    let other = Arc::new(MemoryAuditLedger::default());
    // 相同合法事件在另一偏移得到自洽但不同的回执，不是原外部证据。
    other
        .append(records.last().unwrap().event().clone())
        .unwrap();
    let manager = Arc::new(
        SessionManager::open(
            &h.h.root,
            config(3600, 7200),
            other,
            h.h.clock.clone(),
            h.h.random.clone(),
        )
        .unwrap(),
    );
    let recovery = Recovery::attach(manager.clone()).unwrap();
    assert_eq!(
        recovery
            .rotate_codes(&request(&owner, &issued, 1))
            .err()
            .unwrap(),
        SessionError::AuditUnavailable
    );
    // 原批十枚完整保留，失败的新批十枚仍受 pending 保护，不可发布或直接丢弃证据。
    assert_eq!(scalar(&manager, "SELECT count(*) FROM recovery_codes"), 20);
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM recovery_codes WHERE generation=json_extract((SELECT current_json FROM recovery_control),'$.operation.generation')"
        ),
        10
    );
    assert_eq!(
        scalar(&manager, "SELECT used_mask FROM recovery_control"),
        0
    );
    h.totp
        .require_l3(
            &request(&owner, &issued, 1),
            SensitiveAction::ManageRecoveryCodes,
        )
        .unwrap();
}
