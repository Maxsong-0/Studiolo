//! 首次登记的窄状态投影；显示状态不消费 OTP、补记决定或激活因子。

use super::*;
use std::sync::atomic::Ordering;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn ready_state_is_not_enrollment_and_does_not_require_external_audit() {
    let h = TotpHarness::new(3600, 7200);
    h.h.audit.fail.store(true, Ordering::SeqCst);
    h.h.random.fail.store(true, Ordering::SeqCst);
    assert_eq!(h.enrollment.state().unwrap(), TotpEnrollmentState::Ready);
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_factor"), 0);
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_rates"), 0);
    assert!(h.h.audit.query(&AuditFilter::default()).unwrap().is_empty());
}

#[test]
fn pending_state_keeps_consumption_until_explicit_audited_expiry_cleanup() {
    let h = TotpHarness::new(3600, 7200);
    h.h.audit.fail_outcome.store(true, Ordering::SeqCst);
    assert_eq!(
        h.enrollment.enroll(code(START)).unwrap_err(),
        SessionError::AuditUnavailable
    );
    let last_step = scalar(&h.h.manager, "SELECT last_step FROM totp_factor");
    let before = h.h.audit.query(&AuditFilter::default()).unwrap();
    for now in [START, START + 300_000] {
        h.h.clock.set(now);
        assert_eq!(h.enrollment.state().unwrap(), TotpEnrollmentState::Pending);
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 1);
        assert_eq!(
            scalar(
                &h.h.manager,
                "SELECT count(*) FROM totp_factor WHERE enrollment_json IS NOT NULL"
            ),
            0
        );
        assert_eq!(h.h.audit.query(&AuditFilter::default()).unwrap(), before);
    }
    h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
    let (_, totp, enrollment) = h.reopen();
    assert_eq!(enrollment.state().unwrap(), TotpEnrollmentState::Pending);
    totp.flush_audit().unwrap();
    assert_eq!(enrollment.state().unwrap(), TotpEnrollmentState::Ready);
    assert_eq!(
        scalar(&h.h.manager, "SELECT last_step FROM totp_factor"),
        last_step
    );
    enrollment.enroll(code(START + 300_000)).unwrap();
    assert_eq!(enrollment.state().unwrap(), TotpEnrollmentState::Enrolled);
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 0);
}

#[test]
fn acknowledged_live_pending_is_not_reported_ready_or_activated() {
    let h = TotpHarness::new(3600, 7200);
    h.h.audit.fail_outcome.store(true, Ordering::SeqCst);
    assert!(h.enrollment.enroll(code(START)).is_err());
    h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
    h.totp.flush_audit().unwrap();
    assert_eq!(h.enrollment.state().unwrap(), TotpEnrollmentState::Pending);
    assert_eq!(
        h.enrollment.enroll(code(START)).unwrap_err(),
        SessionError::OperationPending
    );
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM totp_factor WHERE enrollment_json IS NOT NULL"
        ),
        0
    );
}

#[test]
fn already_enrolled_state_is_not_an_idempotent_enrollment_success() {
    let h = TotpHarness::enrolled(3600, 7200);
    let before = h.h.audit.query(&AuditFilter::default()).unwrap();
    h.h.audit.fail.store(true, Ordering::SeqCst);
    assert_eq!(h.enrollment.state().unwrap(), TotpEnrollmentState::Enrolled);
    assert_eq!(
        h.enrollment.enroll(code(NEXT)).unwrap_err(),
        SessionError::AlreadyEnrolled
    );
    assert_eq!(h.h.audit.query(&AuditFilter::default()).unwrap(), before);
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 0);
}

#[test]
fn ready_projection_cannot_reserve_first_enrollment_against_another_instance() {
    let h = TotpHarness::new(3600, 7200);
    assert_eq!(h.enrollment.state().unwrap(), TotpEnrollmentState::Ready);
    let (_, _, second) = h.reopen();
    second.enroll(code(START)).unwrap();
    assert_eq!(h.enrollment.state().unwrap(), TotpEnrollmentState::Enrolled);
    assert_eq!(
        h.enrollment.enroll(code(START)).unwrap_err(),
        SessionError::AlreadyEnrolled
    );
    assert_eq!(
        h.h.audit
            .query(&AuditFilter::default().with_action("totp.enrolled"))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn state_rechecks_seed_binding_clock_and_persisted_receipts() {
    let h = TotpHarness::new(3600, 7200);
    let (_, wrong) = Totp::attach(
        h.h.manager.clone(),
        TotpSeed::new(SEED_REF.into(), [8; 20]).unwrap(),
    )
    .unwrap();
    h.enrollment.enroll(code(START)).unwrap();
    assert_eq!(wrong.state().unwrap_err(), SessionError::Configuration);
    h.h.clock.set(START + 1);
    assert_eq!(h.enrollment.state().unwrap(), TotpEnrollmentState::Enrolled);
    h.h.clock.set(START);
    assert_eq!(
        h.enrollment.state().unwrap_err(),
        SessionError::ClockRollback
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT clock_floor FROM auth_meta"),
        START + 1
    );
    h.h.clock.set(START + 1);
    h.h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute_batch("UPDATE totp_factor SET outcome_receipt='{}'")
        .unwrap();
    assert_eq!(
        h.enrollment.state().unwrap_err(),
        SessionError::CorruptStore
    );
}

#[test]
fn state_preserves_storage_permission_and_session_expiry_checks() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let h = TotpHarness::new(10, 100);
    let issued = h.h.manager.issue(&identity()).unwrap();
    h.h.clock.set(START + 10_000);
    assert_eq!(h.enrollment.state().unwrap(), TotpEnrollmentState::Ready);
    assert_eq!(
        h.h.manager.check(&identity(), &issued.token).unwrap_err(),
        SessionError::Rejected
    );
    let path = h.h.root.join("auth.sqlite3");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        h.enrollment.state().unwrap_err(),
        SessionError::UnsafeStorage
    );
}
