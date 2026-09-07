//! 真实临时 SQLite、受控时间、随机源与具有精确幂等的故障审计。

use super::*;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    },
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};

pub(super) struct TestClock(pub AtomicI64);
impl SessionClock for TestClock {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}
impl TestClock {
    pub fn set(&self, time: i64) {
        self.0.store(time, Ordering::SeqCst);
    }
}

pub(super) struct TestRandom {
    pub next: AtomicU64,
    pub fail: AtomicBool,
}
impl Default for TestRandom {
    fn default() -> Self {
        Self {
            next: AtomicU64::new(1),
            fail: AtomicBool::new(false),
        }
    }
}
impl SessionRandom for TestRandom {
    fn fill(&self, output: &mut [u8]) -> Result<(), SessionError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(SessionError::RandomUnavailable);
        }
        let value = self.next.fetch_add(1, Ordering::SeqCst);
        let bytes = Sha256::digest(value.to_le_bytes());
        output.copy_from_slice(&bytes[..output.len()]);
        Ok(())
    }
}

type Hook = Arc<dyn Fn(&AuditEvent) + Send + Sync>;
#[derive(Default)]
pub(super) struct FaultAudit {
    pub inner: MemoryAuditLedger,
    pub fail: AtomicBool,
    pub fail_intent: AtomicBool,
    pub fail_outcome: AtomicBool,
    pub after_append: AtomicBool,
    pub hook: Mutex<Option<Hook>>,
}
impl AuditLedger for FaultAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let hook = self.hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(&event);
        }
        if self.fail.load(Ordering::SeqCst)
            || (event.phase() == AuditPhase::Intent && self.fail_intent.load(Ordering::SeqCst))
            || (event.phase() == AuditPhase::Outcome && self.fail_outcome.load(Ordering::SeqCst))
        {
            return Err(AuditError::Unavailable);
        }
        let result = self.inner.append(event)?;
        if self.after_append.load(Ordering::SeqCst) {
            return Err(AuditError::Unavailable);
        }
        Ok(result)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}

pub(super) fn config(idle: u64, absolute: u64) -> SessionConfig {
    SessionConfig::new(
        OwnerBinding::new(
            "https://studiolo.cloudflareaccess.com".into(),
            "example-audience".into(),
            "fictional-person".into(),
            "maxsong@example.com".into(),
        )
        .unwrap(),
        SessionPolicy::new(idle, absolute).unwrap(),
    )
}

pub(super) fn directory() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

pub(super) struct Harness {
    pub _directory: tempfile::TempDir,
    pub root: PathBuf,
    pub manager: Arc<SessionManager>,
    pub clock: Arc<TestClock>,
    pub random: Arc<TestRandom>,
    pub audit: Arc<FaultAudit>,
}
impl Harness {
    pub fn new(idle: u64, absolute: u64) -> Self {
        let directory = directory();
        let root = directory.path().canonicalize().unwrap();
        let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
        let random = Arc::new(TestRandom::default());
        let audit = Arc::new(FaultAudit::default());
        let manager = Arc::new(
            SessionManager::open(
                &root,
                config(idle, absolute),
                audit.clone(),
                clock.clone(),
                random.clone(),
            )
            .unwrap(),
        );
        Self {
            _directory: directory,
            root,
            manager,
            clock,
            random,
            audit,
        }
    }
    pub fn reopen(&self, idle: u64, absolute: u64) -> SessionManager {
        SessionManager::open(
            &self.root,
            config(idle, absolute),
            self.audit.clone(),
            self.clock.clone(),
            self.random.clone(),
        )
        .unwrap()
    }
}

pub(super) fn identity() -> crate::AccessIdentity {
    crate::access::test_identity()
}
pub(super) fn scalar(manager: &SessionManager, query: &str) -> i64 {
    manager
        .database
        .connection
        .lock()
        .unwrap()
        .query_row(query, [], |r| r.get(0))
        .unwrap()
}
