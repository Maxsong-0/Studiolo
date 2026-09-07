//! 两个真实私有 SQLite 关闭重开；外部回执与最终 Auth 提交故障不能伪造恢复成功。

use super::durable_support::DurableRecovery;
use super::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{Arc, atomic::Ordering},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};

#[test]
fn real_stores_reopen_before_and_after_recovery_without_preserving_old_authority() {
    let h = DurableRecovery::new();
    let (issued, codes) = h.ready();
    let before = h.audit.query(&AuditFilter::default()).unwrap();
    let h = h.reopen();
    assert_eq!(h.audit.query(&AuditFilter::default()).unwrap(), before);
    h.clock.time.set(CHANGE);
    drop(h.replace(&issued, &codes, 0, CHANGE).unwrap());
    let h = h.reopen();
    let owner = identity();
    assert_eq!(
        h.manager.check(&owner, &issued.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        Totp::attach(h.manager.clone(), seed(OLD)).err().unwrap(),
        SessionError::Configuration
    );
    let (totp, _) = Totp::attach(h.manager.clone(), seed(NEW)).unwrap();
    let fresh = h.manager.issue(&owner).unwrap();
    let req = request(&owner, &fresh, 1);
    assert_eq!(
        totp.require_l3(&req, SensitiveAction::RestoreVault)
            .unwrap_err(),
        SessionError::ReauthenticationRequired
    );
    assert_eq!(
        totp.step_up(&req, otp(&NEW, CHANGE)).unwrap_err(),
        SessionError::Rejected
    );
    h.clock.time.set(CHANGE + 30_000);
    totp.step_up(&req, otp(&NEW, CHANGE + 30_000)).unwrap();
    let prepared = PreparedTotpFactor::prepare(h.manager.clone(), seed(OLD)).unwrap();
    assert_eq!(
        h.recovery()
            .replace_factor(
                &req,
                copy_code(&codes, 1),
                &prepared,
                otp(&OLD, CHANGE + 30_000)
            )
            .err()
            .unwrap(),
        SessionError::Rejected
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM recovery_codes"), 0);
    for record in h.audit.query(&AuditFilter::default()).unwrap() {
        record.verify_integrity().unwrap();
        assert_eq!(h.audit.append(record.event().clone()).unwrap(), record);
    }
}

#[test]
fn missing_or_reordered_real_ledger_cannot_replace_original_batch_receipts() {
    for reordered in [false, true] {
        let h = DurableRecovery::new();
        let (issued, _) = h.ready();
        let before = h.audit.query(&AuditFilter::default()).unwrap();
        let other_dir = directory();
        let other =
            Arc::new(SqliteAuditLedger::open(&other_dir.path().canonicalize().unwrap()).unwrap());
        if reordered {
            let original = before.last().unwrap();
            let replacement = other.append(original.event().clone()).unwrap();
            replacement.verify_integrity().unwrap();
            assert_ne!(replacement.offset(), original.offset());
        } else {
            assert!(other.query(&AuditFilter::default()).unwrap().is_empty());
        }
        let manager = Arc::new(
            SessionManager::open(
                &h.auth_dir.path().canonicalize().unwrap(),
                config(3600, 7200),
                other,
                h.clock.clone(),
                h.random.clone(),
            )
            .unwrap(),
        );
        let recovery = Recovery::attach(manager.clone()).unwrap();
        assert_eq!(
            recovery
                .rotate_codes(&request(&identity(), &issued, 1))
                .err()
                .unwrap(),
            SessionError::AuditUnavailable
        );
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
        assert_eq!(h.audit.query(&AuditFilter::default()).unwrap(), before);
    }
}

#[test]
fn final_auth_commit_failure_after_both_durable_acks_never_switches_on_flush() {
    let h = DurableRecovery::new();
    let (issued, codes) = h.ready();
    let epoch = scalar(&h.manager, "SELECT epoch FROM auth_meta");
    h.clock.time.set(CHANGE);
    h.audit.inject.store(true, Ordering::SeqCst);
    assert_eq!(
        h.replace(&issued, &codes, 0, CHANGE).err().unwrap(),
        SessionError::UnsafeStorage
    );
    assert!(h.clock.fired.load(Ordering::SeqCst));
    fs::set_permissions(&h.clock.file, fs::Permissions::from_mode(0o600)).unwrap();
    // 双 ACK 已经提交；失败点不是前面的审计追加或 ACK 写入。
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM recovery_pending WHERE intent_receipt IS NOT NULL AND outcome_receipt IS NOT NULL"
        ),
        1
    );
    assert_eq!(scalar(&h.manager, "SELECT epoch FROM auth_meta"), epoch);
    assert_eq!(
        scalar(&h.manager, "SELECT used_mask FROM recovery_control"),
        1
    );
    let receipts = h
        .audit
        .query(&AuditFilter::default().with_action("totp.recovered"))
        .unwrap();
    assert_eq!(receipts.len(), 2);
    let h = h.reopen();
    assert_eq!(h.recovery().flush_audit().unwrap(), 0);
    assert_eq!(
        h.audit
            .query(&AuditFilter::default().with_action("totp.recovered"))
            .unwrap(),
        receipts
    );
    h.manager.check(&identity(), &issued.token).unwrap();
    assert!(Totp::attach(h.manager.clone(), seed(OLD)).is_ok());
    assert_eq!(
        Totp::attach(h.manager.clone(), seed(NEW)).err().unwrap(),
        SessionError::Configuration
    );
    assert_eq!(
        scalar(&h.manager, "SELECT count(*) FROM recovery_pending"),
        1
    );
    h.clock.time.set(CHANGE + 300_000);
    assert_eq!(h.recovery().flush_audit().unwrap(), 0);
    assert_eq!(
        scalar(&h.manager, "SELECT count(*) FROM recovery_pending"),
        0
    );
    assert_eq!(
        h.replace(&issued, &codes, 0, CHANGE + 300_000)
            .err()
            .unwrap(),
        SessionError::Rejected
    );
    drop(h.replace(&issued, &codes, 1, CHANGE + 300_000).unwrap());
    let h = h.reopen();
    assert_eq!(
        h.manager.check(&identity(), &issued.token).unwrap_err(),
        SessionError::Rejected
    );
    assert!(Totp::attach(h.manager.clone(), seed(NEW)).is_ok());
}
