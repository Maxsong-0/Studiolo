//! 真实 RS256 身份经持久会话与真实审计完成签发、读取和两库重开。

use super::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, atomic::AtomicI64},
    time::{SystemTime, UNIX_EPOCH},
};
use studiolo_audit_ledger::{
    AuditError, AuditFilter, AuditLedger, AuditPhase, AuditRecord, SqliteAuditLedger,
};

pub(super) struct DurableHarness {
    pub(super) manager: SessionManager,
    pub(super) audit: Arc<SqliteAuditLedger>,
    clock: Arc<TestClock>,
    random: Arc<TestRandom>,
    auth_directory: tempfile::TempDir,
    audit_directory: tempfile::TempDir,
}

impl DurableHarness {
    pub(super) fn new(now: i64) -> Self {
        let auth_directory = directory();
        let audit_directory = directory();
        let auth_root = auth_directory.path().canonicalize().unwrap();
        let audit_root = audit_directory.path().canonicalize().unwrap();
        assert_ne!(auth_root, audit_root);
        let audit = Arc::new(SqliteAuditLedger::open(&audit_root).unwrap());
        let clock = Arc::new(TestClock(AtomicI64::new(now)));
        let random = Arc::new(TestRandom::default());
        let manager = SessionManager::open(
            &auth_root,
            config(60, 300),
            audit.clone(),
            clock.clone(),
            random.clone(),
        )
        .unwrap();
        assert!(auth_root.join("auth.sqlite3").is_file());
        assert!(audit_root.join("audit.sqlite3").is_file());
        assert!(!auth_root.join("audit.sqlite3").exists());
        assert!(!audit_root.join("auth.sqlite3").exists());
        Self {
            manager,
            audit,
            clock,
            random,
            auth_directory,
            audit_directory,
        }
    }

    pub(super) fn audit_file(&self) -> PathBuf {
        self.audit_directory
            .path()
            .canonicalize()
            .unwrap()
            .join("audit.sqlite3")
    }

    pub(super) fn reopen_auth(self) -> Self {
        let Self {
            manager,
            audit,
            clock,
            random,
            auth_directory,
            audit_directory,
        } = self;
        drop(manager);
        let root = auth_directory.path().canonicalize().unwrap();
        let manager = SessionManager::open(
            &root,
            config(60, 300),
            audit.clone(),
            clock.clone(),
            random.clone(),
        )
        .unwrap();
        Self {
            manager,
            audit,
            clock,
            random,
            auth_directory,
            audit_directory,
        }
    }

    pub(super) fn reopen_both(self) -> Self {
        let Self {
            manager,
            audit,
            clock,
            random,
            auth_directory,
            audit_directory,
        } = self;
        drop(manager);
        drop(audit);
        let audit = Arc::new(
            SqliteAuditLedger::open(&audit_directory.path().canonicalize().unwrap()).unwrap(),
        );
        let root = auth_directory.path().canonicalize().unwrap();
        let manager = SessionManager::open(
            &root,
            config(60, 300),
            audit.clone(),
            clock.clone(),
            random.clone(),
        )
        .unwrap();
        Self {
            manager,
            audit,
            clock,
            random,
            auth_directory,
            audit_directory,
        }
    }
}

pub(super) fn stored_text(manager: &SessionManager, query: &str) -> Vec<String> {
    let connection = manager.database.connection.lock().unwrap();
    let mut statement = connection.prepare(query).unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[tokio::test]
async fn real_signed_identity_issues_and_checks_across_both_durable_stores() {
    let identity = crate::access::test_signed_identity().await;
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let harness = DurableHarness::new(now);
    let issued = harness.manager.issue(&identity).unwrap();
    assert_eq!(
        harness.manager.check(&identity, &issued.token).unwrap(),
        issued.session
    );
    let records = harness.audit.query(&AuditFilter::default()).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
    assert_eq!(records[0].event().event_id(), records[1].event().event_id());
    assert_eq!(records[1].previous_hash(), records[0].record_hash());
    for record in &records {
        record.verify_integrity().unwrap();
        assert_eq!(record.event().action(), "session.issue");
    }
    let stored = stored_text(
        &harness.manager,
        "SELECT grant_receipt FROM sessions WHERE state='active'",
    );
    assert_eq!(stored.len(), 1);
    assert_eq!(
        serde_json::from_str::<AuditRecord>(&stored[0]).unwrap(),
        records[1]
    );
    let reopened = harness.reopen_both();
    assert_eq!(
        reopened.manager.check(&identity, &issued.token).unwrap(),
        issued.session
    );
    assert_eq!(
        reopened.audit.query(&AuditFilter::default()).unwrap(),
        records
    );
    assert_eq!(reopened.manager.flush_audit(256).unwrap(), 0);
    assert_eq!(
        reopened.manager.sessions(&identity, &issued.token).unwrap(),
        vec![issued.session]
    );
}

#[test]
fn real_audit_unavailability_blocks_new_grants_but_not_valid_reads() {
    let harness = DurableHarness::new(1_000_000);
    let identity = identity();
    let issued = harness.manager.issue(&identity).unwrap();
    let before = harness.audit.query(&AuditFilter::default()).unwrap();
    fs::set_permissions(harness.audit_file(), fs::Permissions::from_mode(0o400)).unwrap();
    assert_eq!(
        harness.audit.query(&AuditFilter::default()).unwrap_err(),
        AuditError::UnsafeStorage
    );
    assert_eq!(
        harness.manager.issue(&identity).unwrap_err(),
        SessionError::AuditUnavailable
    );
    assert_eq!(scalar(&harness.manager, "SELECT count(*) FROM sessions"), 1);
    assert_eq!(
        harness
            .manager
            .check(&identity, &issued.token)
            .unwrap()
            .state,
        SessionState::Active
    );
    fs::set_permissions(harness.audit_file(), fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        harness.audit.query(&AuditFilter::default()).unwrap(),
        before
    );
}
