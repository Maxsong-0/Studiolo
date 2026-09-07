//! 启动预检的公开 Interface：不迁移、不登记、不补账，也不签发探针会话。

use super::*;
use super::{http_ready_support::*, http_support::*};
use std::sync::{Arc, atomic::Ordering};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, MemoryAuditLedger};

#[test]
fn readiness_rejects_v2_without_migrating_or_querying_audit() {
    let h = fixture(2, 10, 100);
    let audit = Arc::new(ReadAudit::new(h.audit.clone()));
    let manager = open(&h, audit.clone());
    assert_eq!(
        manager.require_http_ready(),
        Err(SessionError::MigrationRequired)
    );
    assert_eq!(scalar(&manager, "PRAGMA user_version"), 2);
    assert_eq!(audit.queries.load(Ordering::SeqCst), 0);
    assert_eq!(audit.appends.load(Ordering::SeqCst), 0);
}

#[test]
fn readiness_rejects_unregistered_v3_v4_and_preserves_observed_floor() {
    for version in [3, 4] {
        let h = fixture(version, 10, 100);
        let audit = Arc::new(ReadAudit::new(h.audit.clone()));
        let manager = open(&h, audit.clone());
        h.clock.set(START + 1000);
        assert_eq!(
            manager.require_http_ready(),
            Err(SessionError::FactorUnavailable)
        );
        assert_eq!(
            scalar(&manager, "SELECT clock_floor FROM auth_meta"),
            START + 1000
        );
        assert_eq!(audit.queries.load(Ordering::SeqCst), 0);
        assert_eq!(scalar(&manager, "SELECT count(*) FROM sessions"), 0);
    }
}

#[test]
fn readiness_rejects_pending_only_even_after_original_receipts_are_delivered() {
    for version in [3, 4] {
        let h = fixture(version, 10, 100);
        h.audit.fail_outcome.store(true, Ordering::SeqCst);
        assert_eq!(
            enrollment(&h).enroll(code()).unwrap_err(),
            SessionError::AuditUnavailable
        );
        let audit = Arc::new(ReadAudit::new(h.audit.clone()));
        let manager = open(&h, audit.clone());
        assert_eq!(
            manager.require_http_ready(),
            Err(SessionError::FactorUnavailable)
        );
        h.audit.fail_outcome.store(false, Ordering::SeqCst);
        Totp::attach(h.manager.clone(), seed())
            .unwrap()
            .0
            .flush_audit()
            .unwrap();
        assert_eq!(
            manager.require_http_ready(),
            Err(SessionError::FactorUnavailable)
        );
        assert_eq!(audit.queries.load(Ordering::SeqCst), 0);
        assert_eq!(audit.appends.load(Ordering::SeqCst), 0);
        assert_eq!(scalar(&manager, "SELECT count(*) FROM totp_pending"), 1);
    }
}

#[test]
fn readiness_accepts_confirmed_v3_v4_without_consumption_issuance_or_append() {
    for version in [3, 4] {
        let h = enrolled(version, 10, 100);
        let records = h.audit.query(&AuditFilter::default()).unwrap();
        let audit = Arc::new(ReadAudit::new(h.audit.clone()));
        let manager = open(&h, audit.clone());
        let step = scalar(&manager, "SELECT last_step FROM totp_factor");
        for _ in 0..2 {
            manager.require_http_ready().unwrap();
        }
        assert_eq!(audit.queries.load(Ordering::SeqCst), 4);
        assert_eq!(audit.appends.load(Ordering::SeqCst), 0);
        assert_eq!(scalar(&manager, "SELECT last_step FROM totp_factor"), step);
        assert_eq!(
            scalar(&manager, "SELECT next_operation FROM totp_control"),
            2
        );
        assert_eq!(scalar(&manager, "SELECT sum(attempts) FROM totp_rates"), 2);
        assert_eq!(
            scalar(&manager, "SELECT next_issuance FROM credential_state"),
            1
        );
        assert_eq!(scalar(&manager, "SELECT count(*) FROM sessions"), 0);
        assert_eq!(scalar(&manager, "SELECT sum(total) FROM auth_denials"), 0);
        assert_eq!(h.audit.query(&AuditFilter::default()).unwrap(), records);
    }
}

#[test]
fn readiness_external_query_errors_or_nonexact_receipts_do_not_poison_auth() {
    for fault in [
        Fault::Query,
        Fault::MissingIntent,
        Fault::MissingOutcome,
        Fault::Changed,
        Fault::Duplicate,
    ] {
        let h = enrolled(4, 10, 100);
        let audit = Arc::new(ReadAudit::new(h.audit.clone()));
        audit.fault.store(fault as u8, Ordering::SeqCst);
        let manager = open(&h, audit.clone());
        assert_eq!(
            manager.require_http_ready(),
            Err(SessionError::AuditUnavailable),
            "{fault:?}"
        );
        assert_eq!(audit.appends.load(Ordering::SeqCst), 0);
        drop(manager);
        h.reopen(10, 100).require_http_ready().unwrap();
    }
}

#[test]
fn readiness_wrong_legal_empty_ledger_is_not_filled_with_original_events() {
    let h = enrolled(4, 10, 100);
    let empty = Arc::new(MemoryAuditLedger::default());
    let audit = Arc::new(ReadAudit::new(empty.clone()));
    assert_eq!(
        open(&h, audit.clone()).require_http_ready(),
        Err(SessionError::AuditUnavailable)
    );
    assert_eq!(audit.appends.load(Ordering::SeqCst), 0);
    assert!(empty.query(&AuditFilter::default()).unwrap().is_empty());
}

#[test]
fn readiness_local_damaged_proof_fails_before_external_query() {
    let h = enrolled(4, 10, 100);
    let audit = Arc::new(ReadAudit::new(h.audit.clone()));
    let manager = open(&h, audit.clone());
    h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE totp_factor SET outcome_receipt='{}'", [])
        .unwrap();
    assert_eq!(
        manager.require_http_ready(),
        Err(SessionError::CorruptStore)
    );
    assert_eq!(audit.queries.load(Ordering::SeqCst), 0);
    assert_eq!(audit.appends.load(Ordering::SeqCst), 0);
}
