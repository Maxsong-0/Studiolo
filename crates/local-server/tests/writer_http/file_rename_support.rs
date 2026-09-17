//! 只替换真实依赖 adapter；没有产品故障 setter、短期限或跳过身份的开关。
use crate::{fixture::Fixture, support as http, test_support};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
    },
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, SqliteAuditLedger,
};
use studiolo_auth::{OsSessionRandom, SessionClock, SessionError, SessionManager};
use studiolo_vault_broker::ReadOnlyVault;

pub(super) struct OutcomeFailure(SqliteAuditLedger);
impl OutcomeFailure {
    pub(super) fn open(path: &Path) -> Self {
        Self(SqliteAuditLedger::open_existing(path).unwrap())
    }
}
impl AuditLedger for OutcomeFailure {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.phase() == AuditPhase::Outcome
            && event.payload().as_value().get("file_operation").is_some()
        {
            return Err(AuditError::Unavailable);
        }
        self.0.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.0.query(filter)
    }
}

enum Change {
    Bytes(PathBuf, Vec<u8>),
    Expire(i64),
}
pub(super) struct AdmissionClock {
    now: AtomicI64,
    calls: AtomicUsize,
    armed: AtomicBool,
    action: Mutex<Option<(usize, Change)>>,
}
impl AdmissionClock {
    pub(super) fn new(now: i64) -> Self {
        Self {
            now: AtomicI64::new(now),
            calls: AtomicUsize::new(0),
            armed: AtomicBool::new(false),
            action: Mutex::new(None),
        }
    }
    pub(super) fn replace(&self, path: PathBuf, bytes: Vec<u8>) {
        // HTTP Auth 第一次、file_input_access 第二次、admission 内 verify 第三次。
        self.arm(3, Change::Bytes(path, bytes));
    }
    pub(super) fn expire(&self, now: i64) {
        // 第四/五次是 prepare 前 gate；第六/七次是 Intent ACK 后的最终 gate。
        self.arm(7, Change::Expire(now));
    }
    fn arm(&self, call: usize, change: Change) {
        self.calls.store(0, Ordering::SeqCst);
        *self.action.lock().unwrap() = Some((call, change));
        self.armed.store(true, Ordering::SeqCst);
    }
    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    pub(super) fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
        assert!(
            self.action.lock().unwrap().is_none(),
            "必须到达指定的真实调用"
        );
    }
}
impl SessionClock for AdmissionClock {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        if self.armed.load(Ordering::SeqCst) {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            let mut action = self.action.lock().unwrap();
            if action.as_ref().is_some_and(|(at, _)| *at == call) {
                match action.take().unwrap().1 {
                    Change::Bytes(path, bytes) => fs::write(path, bytes).unwrap(),
                    Change::Expire(now) => self.now.store(now, Ordering::SeqCst),
                }
            }
        }
        Ok(self.now.load(Ordering::SeqCst))
    }
}

pub(super) async fn clock(f: &mut Fixture, clock: Arc<AdmissionClock>) {
    let sessions = Arc::new(
        SessionManager::open_existing(
            &f.http.auth,
            http::config(),
            Arc::new(SqliteAuditLedger::open_existing(&f.http.audit).unwrap()),
            clock,
            Arc::new(OsSessionRandom),
        )
        .unwrap(),
    );
    f.http.app = studiolo_local_server::router_with_canonical_writer(
        f.owner.clone(),
        ReadOnlyVault::open(&f.http.root.join("web")).unwrap(),
        "重命名真实时钟夹具".into(),
        4319,
        test_support::verifier().await,
        sessions,
        None,
    )
    .unwrap();
}
