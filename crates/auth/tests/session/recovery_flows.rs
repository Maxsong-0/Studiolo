//! 正常调用自然闭合：恢复只切换因子，不返回通用权限或保活旧会话。

use super::*;
use std::{
    collections::HashSet,
    sync::{Arc, atomic::Ordering},
};

#[test]
fn code_management_requires_current_session_l3_and_rotation_retires_old_batch() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let first = h.h.manager.issue(&owner).unwrap();
    let second = h.h.manager.issue(&owner).unwrap();
    assert_eq!(
        h.recovery
            .rotate_codes(&request(&owner, &first, 1))
            .err()
            .unwrap(),
        SessionError::ReauthenticationRequired
    );
    h.totp
        .step_up(&request(&owner, &first, 1), otp(&OLD, NEXT))
        .unwrap();
    assert_eq!(
        h.recovery
            .rotate_codes(&request(&owner, &second, 1))
            .err()
            .unwrap(),
        SessionError::ReauthenticationRequired
    );
    let old = h
        .recovery
        .rotate_codes(&request(&owner, &first, 1))
        .unwrap();
    let current = h
        .recovery
        .rotate_codes(&request(&owner, &first, 1))
        .unwrap();
    assert_eq!(current.codes().len(), 10);
    let unique: HashSet<_> = current.codes().iter().map(|c| c.expose_secret()).collect();
    assert_eq!(unique.len(), 10);
    for (slot, code) in current.codes().iter().enumerate() {
        assert_eq!(code.expose_secret().len(), 55);
        assert_eq!(&code.expose_secret()[20..22], format!("{slot:02x}"));
        assert!(RecoveryCode::parse(code.expose_secret().to_owned()).is_ok());
        assert!(
            !old.codes()
                .iter()
                .any(|prior| prior.expose_secret() == code.expose_secret())
        );
    }
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_codes"),
        10
    );
    h.h.clock.set(CHANGE);
    let prepared = h.prepared();
    assert_eq!(
        h.recovery
            .replace_factor(
                &request(&owner, &first, 1),
                copy_code(&old, 0),
                &prepared,
                otp(&NEW, CHANGE)
            )
            .err()
            .unwrap(),
        SessionError::Rejected
    );
    h.recovery
        .replace_factor(
            &request(&owner, &first, 1),
            copy_code(&current, 0),
            &prepared,
            otp(&NEW, CHANGE),
        )
        .unwrap();
}

#[test]
fn complete_recovery_revokes_every_old_session_and_requires_fresh_login_and_fresh_otp() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (first, codes) = h.ready();
    let second = h.h.manager.issue(&owner).unwrap();
    h.h.clock.set(NEXT + 30_000);
    h.totp
        .step_up(&request(&owner, &second, 2), otp(&OLD, NEXT + 30_000))
        .unwrap();
    let prepared = h.prepared();
    h.h.clock.set(CHANGE);
    let replacement = h
        .recovery
        .replace_factor(
            &request(&owner, &first, 1),
            copy_code(&codes, 0),
            &prepared,
            otp(&NEW, CHANGE),
        )
        .unwrap();
    for issued in [&first, &second] {
        assert_eq!(
            h.h.manager.check(&owner, &issued.token).unwrap_err(),
            SessionError::Rejected
        );
        assert!(
            h.totp
                .require_l3(&request(&owner, issued, 1), SensitiveAction::RestoreVault)
                .is_err()
        );
    }
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 0);
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_codes"),
        0
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        0
    );
    assert_eq!(
        Totp::attach(h.h.manager.clone(), seed(OLD)).err().unwrap(),
        SessionError::Configuration
    );
    let fresh = h.h.manager.issue(&owner).unwrap();
    let req = request(&owner, &fresh, 1);
    assert_eq!(
        replacement
            .require_l3(&req, SensitiveAction::ManageRecoveryCodes)
            .unwrap_err(),
        SessionError::ReauthenticationRequired
    );
    assert_eq!(
        replacement.step_up(&req, otp(&NEW, CHANGE)).unwrap_err(),
        SessionError::Rejected
    );
    h.h.clock.set(CHANGE + 30_000);
    assert!(h.totp.step_up(&req, otp(&OLD, CHANGE + 30_000)).is_err());
    replacement
        .step_up(&req, otp(&NEW, CHANGE + 30_000))
        .unwrap();
    let renewed = h.recovery.rotate_codes(&req).unwrap();
    assert_eq!(renewed.codes().len(), 10);
    let stale = h.recovery.replace_factor(
        &req,
        copy_code(&codes, 1),
        &prepared,
        otp(&NEW, CHANGE + 30_000),
    );
    assert!(stale.is_err());
}

#[test]
fn prepared_factor_rejects_same_secret_even_with_new_reference_and_other_store() {
    let h = RecoveryHarness::new(3600, 7200);
    let same = TotpSeed::new("test-only-renamed-old".into(), OLD).unwrap();
    assert_eq!(
        PreparedTotpFactor::prepare(h.h.manager.clone(), same)
            .err()
            .unwrap(),
        SessionError::Configuration
    );
    // 共享基础夹具的随机序列默认相同；独立 store 必须显式使用不同随机输入。
    let other = directory();
    let other_root = other.path().canonicalize().unwrap();
    let random = Arc::new(TestRandom::default());
    random.next.store(1000, Ordering::SeqCst);
    let manager = Arc::new(
        SessionManager::migrate_v3(
            &other_root,
            config(3600, 7200),
            h.h.audit.clone(),
            h.h.clock.clone(),
            random,
        )
        .unwrap(),
    );
    let (_, enrollment) = Totp::attach(manager.clone(), seed(OLD)).unwrap();
    enrollment.enroll(otp(&OLD, NEXT)).unwrap();
    let foreign = PreparedTotpFactor::prepare(manager, seed(NEW)).unwrap();
    let (issued, codes) = h.ready();
    h.h.clock.set(CHANGE);
    assert!(
        h.recovery
            .replace_factor(
                &request(&identity(), &issued, 1),
                copy_code(&codes, 0),
                &foreign,
                otp(&NEW, CHANGE)
            )
            .is_err()
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT used_mask FROM recovery_control"),
        0
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        0
    );
}
