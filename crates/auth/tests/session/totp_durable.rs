//! 因子与审计均使用真实 SQLite；全部句柄关闭后验证保活、消费锚点与故障补记。

use super::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, atomic::AtomicI64},
};
use studiolo_audit_ledger::{AuditError, AuditFilter, AuditLedger, AuditRecord, SqliteAuditLedger};

struct DurableTotp {
    auth_directory: tempfile::TempDir,
    audit_directory: tempfile::TempDir,
    manager: Arc<SessionManager>,
    totp: Totp,
    enrollment: TotpEnrollment,
    audit: Arc<SqliteAuditLedger>,
    clock: Arc<TestClock>,
    random: Arc<TestRandom>,
}

impl DurableTotp {
    fn enrolled() -> Self {
        let auth_directory = directory();
        let audit_directory = directory();
        let root = auth_directory.path().canonicalize().unwrap();
        let audit_root = audit_directory.path().canonicalize().unwrap();
        assert_ne!(root, audit_root);
        for path in [&root, &audit_root] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let audit = Arc::new(SqliteAuditLedger::open(&audit_root).unwrap());
        let clock = Arc::new(TestClock(AtomicI64::new(START)));
        let random = Arc::new(TestRandom::default());
        let manager = Arc::new(
            SessionManager::migrate_v2(
                &root,
                config(3600, 7200),
                audit.clone(),
                clock.clone(),
                random.clone(),
            )
            .unwrap(),
        );
        let (totp, enrollment) = Totp::attach(manager.clone(), seed()).unwrap();
        enrollment.enroll(code(START)).unwrap();
        clock.set(NEXT);
        Self {
            auth_directory,
            audit_directory,
            manager,
            totp,
            enrollment,
            audit,
            clock,
            random,
        }
    }

    fn reopen(self) -> Self {
        let Self {
            auth_directory,
            audit_directory,
            manager,
            totp,
            enrollment,
            audit,
            clock,
            random,
        } = self;
        let old_manager = Arc::downgrade(&manager);
        let old_audit = Arc::downgrade(&audit);
        drop(totp);
        drop(enrollment);
        drop(manager);
        drop(audit);
        // 不能让因子句柄或 Arc 中残留的连接冒充真正磁盘重开。
        assert!(old_manager.upgrade().is_none());
        assert!(old_audit.upgrade().is_none());
        let audit = Arc::new(
            SqliteAuditLedger::open(&audit_directory.path().canonicalize().unwrap()).unwrap(),
        );
        let manager = Arc::new(
            SessionManager::open(
                &auth_directory.path().canonicalize().unwrap(),
                config(3600, 7200),
                audit.clone(),
                clock.clone(),
                random.clone(),
            )
            .unwrap(),
        );
        let (totp, enrollment) = Totp::attach(manager.clone(), seed()).unwrap();
        Self {
            auth_directory,
            audit_directory,
            manager,
            totp,
            enrollment,
            audit,
            clock,
            random,
        }
    }

    fn audit_file(&self) -> PathBuf {
        self.audit_directory
            .path()
            .canonicalize()
            .unwrap()
            .join("audit.sqlite3")
    }

    fn count(&self, query: &str) -> i64 {
        scalar(&self.manager, query)
    }

    fn elevate(
        &self,
        issued: &IssuedSession,
        now: i64,
    ) -> Result<RecentAuthentication, SessionError> {
        self.clock.set(now);
        self.totp
            .step_up(&request(&identity(), issued, 1), code(now))
    }

    fn l3(&self, issued: &IssuedSession) -> Result<(), SessionError> {
        self.totp.require_l3(
            &request(&identity(), issued, 1),
            SensitiveAction::RestoreVault,
        )
    }
}

#[test]
fn enrollment_and_recent_authentication_survive_closing_both_sqlite_adapters() {
    let h = DurableTotp::enrolled();
    let issued = h.manager.issue(&identity()).unwrap();
    let proof = h.elevate(&issued, NEXT).unwrap();
    let before = h.audit.query(&AuditFilter::default()).unwrap();
    let h = h.reopen();
    assert_eq!(h.count("PRAGMA user_version"), 3);
    assert_eq!(h.audit.query(&AuditFilter::default()).unwrap(), before);
    h.l3(&issued).unwrap();
    assert_eq!(
        h.elevate(&issued, NEXT).unwrap_err(),
        SessionError::Rejected
    );
    h.clock.set(proof.expires_at_ms - 1);
    h.l3(&issued).unwrap();
    h.clock.set(proof.expires_at_ms);
    assert_eq!(
        h.l3(&issued).unwrap_err(),
        SessionError::ReauthenticationRequired
    );
    for record in before {
        record.verify_integrity().unwrap();
        assert_eq!(h.audit.append(record.event().clone()).unwrap(), record);
    }
}

#[test]
fn replaced_then_collected_grant_does_not_erase_the_consumption_anchor() {
    let h = DurableTotp::enrolled();
    let owner = identity();
    let controller = h.manager.issue(&owner).unwrap();
    let target = h.manager.issue(&owner).unwrap();
    h.elevate(&target, NEXT).unwrap();
    let earlier = text_rows(&h.manager, "SELECT operation_json FROM totp_grants");
    let latest = NEXT + 60_000;
    h.elevate(&target, latest).unwrap();
    assert_eq!(h.count("SELECT count(*) FROM totp_grants"), 1);
    assert_ne!(
        text_rows(&h.manager, "SELECT operation_json FROM totp_grants"),
        earlier
    );
    let anchor = text_rows(&h.manager, "SELECT consumption_json FROM totp_factor");
    h.manager
        .revoke(
            &owner,
            &controller.token,
            &controller.csrf,
            &target.session.id,
        )
        .unwrap();
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        1
    );
    assert_eq!(h.count("SELECT count(*) FROM totp_grants"), 0);
    assert_eq!(
        text_rows(&h.manager, "SELECT consumption_json FROM totp_factor"),
        anchor
    );
    let h = h.reopen();
    let replacement = h.manager.issue(&owner).unwrap();
    let filter = AuditFilter::default().with_action("totp.step_up");
    let history = h.audit.query(&filter).unwrap();
    assert_eq!(history.len(), 4);
    // 时间仍在刚消费的原步内；拒绝不是旧码自然到期造成的。
    assert_eq!(
        h.elevate(&replacement, latest).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(h.audit.query(&filter).unwrap(), history);
    assert_eq!(
        h.l3(&replacement).unwrap_err(),
        SessionError::ReauthenticationRequired
    );
    assert_eq!(
        h.count("SELECT last_step FROM totp_factor"),
        latest / 30_000
    );
    assert_eq!(
        text_rows(&h.manager, "SELECT consumption_json FROM totp_factor"),
        anchor
    );
    assert_eq!(
        h.manager.check(&owner, &target.token).unwrap_err(),
        SessionError::Rejected
    );
}

#[test]
fn real_audit_permissions_failure_and_fresh_ledger_replay_never_activate_pending() {
    let h = DurableTotp::enrolled();
    let issued = h.manager.issue(&identity()).unwrap();
    let before = h.audit.query(&AuditFilter::default()).unwrap();
    fs::set_permissions(h.audit_file(), fs::Permissions::from_mode(0o400)).unwrap();
    assert_eq!(
        h.audit.query(&AuditFilter::default()).unwrap_err(),
        AuditError::UnsafeStorage
    );
    assert_eq!(
        h.elevate(&issued, NEXT).unwrap_err(),
        SessionError::AuditUnavailable
    );
    assert_eq!(h.count("SELECT count(*) FROM totp_pending"), 1);
    assert_eq!(h.count("SELECT count(*) FROM totp_grants"), 0);
    assert_eq!(
        h.l3(&issued).unwrap_err(),
        SessionError::ReauthenticationRequired
    );
    let pending = text_rows(&h.manager, "SELECT operation_json FROM totp_pending");
    let anchor = text_rows(&h.manager, "SELECT consumption_json FROM totp_factor");
    fs::set_permissions(h.audit_file(), fs::Permissions::from_mode(0o600)).unwrap();
    let h = h.reopen();
    assert_eq!(h.audit.query(&AuditFilter::default()).unwrap(), before);
    assert_eq!(h.totp.flush_audit().unwrap(), 2);
    assert_eq!(
        text_rows(&h.manager, "SELECT operation_json FROM totp_pending"),
        pending
    );
    let delivered = h
        .audit
        .query(&AuditFilter::default().with_action("totp.step_up"))
        .unwrap();
    let receipts: Vec<AuditRecord> = text_rows(&h.manager,
        "SELECT intent_receipt FROM totp_pending UNION ALL SELECT outcome_receipt FROM totp_pending")
        .iter().map(|encoded| serde_json::from_str(encoded).unwrap()).collect();
    assert_eq!(receipts, delivered);
    assert_eq!(delivered.len(), 2);
    let h = h.reopen();
    assert_eq!(h.totp.flush_audit().unwrap(), 0);
    assert_eq!(
        h.l3(&issued).unwrap_err(),
        SessionError::ReauthenticationRequired
    );
    assert_eq!(
        h.elevate(&issued, NEXT).unwrap_err(),
        SessionError::OperationPending
    );
    h.clock.set(NEXT + 300_000);
    assert_eq!(h.totp.flush_audit().unwrap(), 0);
    assert_eq!(h.count("SELECT count(*) FROM totp_pending"), 0);
    assert_eq!(h.count("SELECT count(*) FROM totp_grants"), 0);
    assert_eq!(
        text_rows(&h.manager, "SELECT consumption_json FROM totp_factor"),
        anchor
    );
    // 必须提交另一个新时间步的正确码，恢复审计或再次打开本身不授予权限。
    h.elevate(&issued, NEXT + 300_000).unwrap();
    h.l3(&issued).unwrap();
}
