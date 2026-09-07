//! 本地持久性测试的专用支持；复用既有恢复夹具，不重复注册测试或创建内存审计。

use super::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, SqliteAuditLedger,
};

pub(super) struct CommitClock {
    pub(super) time: TestClock,
    pub(super) file: PathBuf,
    armed: AtomicBool,
    pub(super) fired: AtomicBool,
}

impl SessionClock for CommitClock {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        if self.armed.swap(false, Ordering::SeqCst) {
            // ACK 不采时钟；在最终 Auth 事务内收紧权限，提交前检查必须回滚整个切换。
            fs::set_permissions(&self.file, fs::Permissions::from_mode(0o400)).unwrap();
            self.fired.store(true, Ordering::SeqCst);
        }
        self.time.now_unix_ms()
    }
}

pub(super) struct RealAudit {
    inner: SqliteAuditLedger,
    pub(super) clock: Arc<CommitClock>,
    pub(super) inject: AtomicBool,
}

impl AuditLedger for RealAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let outcome = event.action() == "totp.recovered" && event.phase() == AuditPhase::Outcome;
        let record = self.inner.append(event)?;
        if outcome && self.inject.swap(false, Ordering::SeqCst) {
            self.clock.armed.store(true, Ordering::SeqCst);
        }
        Ok(record)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}

pub(super) struct DurableRecovery {
    pub(super) auth_dir: tempfile::TempDir,
    audit_dir: tempfile::TempDir,
    pub(super) manager: Arc<SessionManager>,
    pub(super) audit: Arc<RealAudit>,
    pub(super) clock: Arc<CommitClock>,
    pub(super) random: Arc<TestRandom>,
}

impl DurableRecovery {
    pub(super) fn new() -> Self {
        let auth_dir = directory();
        let audit_dir = directory();
        let root = auth_dir.path().canonicalize().unwrap();
        let audit_root = audit_dir.path().canonicalize().unwrap();
        assert_ne!(root, audit_root);
        for path in [&root, &audit_root] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let clock = Arc::new(CommitClock {
            time: TestClock(AtomicI64::new(START)),
            file: root.join("auth.sqlite3"),
            armed: AtomicBool::new(false),
            fired: AtomicBool::new(false),
        });
        let random = Arc::new(TestRandom::default());
        let audit = Arc::new(RealAudit {
            inner: SqliteAuditLedger::open(&audit_root).unwrap(),
            clock: clock.clone(),
            inject: AtomicBool::new(false),
        });
        let manager = Arc::new(
            SessionManager::migrate_v3(
                &root,
                config(3600, 7200),
                audit.clone(),
                clock.clone(),
                random.clone(),
            )
            .unwrap(),
        );
        Self {
            auth_dir,
            audit_dir,
            manager,
            audit,
            clock,
            random,
        }
    }

    pub(super) fn reopen(self) -> Self {
        let Self {
            auth_dir,
            audit_dir,
            manager,
            audit,
            clock,
            random,
        } = self;
        let old_manager = Arc::downgrade(&manager);
        let old_audit = Arc::downgrade(&audit);
        drop(manager);
        drop(audit);
        assert!(old_manager.upgrade().is_none());
        assert!(old_audit.upgrade().is_none());
        let audit = Arc::new(RealAudit {
            inner: SqliteAuditLedger::open(&audit_dir.path().canonicalize().unwrap()).unwrap(),
            clock: clock.clone(),
            inject: AtomicBool::new(false),
        });
        let manager = Arc::new(
            SessionManager::open(
                &auth_dir.path().canonicalize().unwrap(),
                config(3600, 7200),
                audit.clone(),
                clock.clone(),
                random.clone(),
            )
            .unwrap(),
        );
        Self {
            auth_dir,
            audit_dir,
            manager,
            audit,
            clock,
            random,
        }
    }

    pub(super) fn recovery(&self) -> Recovery {
        Recovery::attach(self.manager.clone()).unwrap()
    }

    pub(super) fn ready(&self) -> (IssuedSession, RecoveryCodes) {
        let (totp, enrollment) = Totp::attach(self.manager.clone(), seed(OLD)).unwrap();
        enrollment.enroll(otp(&OLD, START)).unwrap();
        self.clock.time.set(NEXT);
        let owner = identity();
        let issued = self.manager.issue(&owner).unwrap();
        let req = request(&owner, &issued, 1);
        totp.step_up(&req, otp(&OLD, NEXT)).unwrap();
        let codes = self.recovery().rotate_codes(&req).unwrap();
        (issued, codes)
    }

    pub(super) fn replace(
        &self,
        issued: &IssuedSession,
        codes: &RecoveryCodes,
        slot: usize,
        now: i64,
    ) -> Result<Totp, SessionError> {
        let prepared = PreparedTotpFactor::prepare(self.manager.clone(), seed(NEW)).unwrap();
        self.recovery().replace_factor(
            &request(&identity(), issued, 1),
            copy_code(codes, slot),
            &prepared,
            otp(&NEW, now),
        )
    }
}
