//! 只在测试装配暂停真实依赖调用，不向产品增加时钟或授权绕过开关。
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};
use studiolo_auth::{SessionClock, SessionError};

#[derive(Default)]
pub(super) struct Pause {
    state: Mutex<(bool, bool)>,
    gate: Condvar,
}
impl Pause {
    fn enter(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.gate.notify_all();
        let waited = self
            .gate
            .wait_timeout_while(state, Duration::from_secs(10), |state| !state.1)
            .unwrap();
        assert!(!waited.1.timed_out(), "真实依赖调用未在期限内释放");
    }
    pub(super) fn wait(&self) {
        let state = self.state.lock().unwrap();
        let waited = self
            .gate
            .wait_timeout_while(state, Duration::from_secs(10), |state| !state.0)
            .unwrap();
        assert!(!waited.1.timed_out(), "未到达指定的真实依赖调用");
    }
    pub(super) fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.gate.notify_all();
    }
}

pub(super) struct FinalClock {
    now: i64,
    armed: Arc<AtomicBool>,
    calls: AtomicUsize,
    pub(super) pause: Pause,
}
impl FinalClock {
    pub(super) fn new(now: i64, armed: Arc<AtomicBool>) -> Self {
        Self {
            now,
            armed,
            calls: AtomicUsize::new(0),
            pause: Pause::default(),
        }
    }
    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl SessionClock for FinalClock {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        // Intent 后先有 file_access 核验，再有持 control 锁的最终 verify。
        if self.armed.load(Ordering::SeqCst) && self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            self.pause.enter();
        }
        Ok(self.now)
    }
}

pub(super) struct IntentAudit {
    ledger: MemoryAuditLedger,
    armed: Arc<AtomicBool>,
}
impl IntentAudit {
    pub(super) fn new(armed: Arc<AtomicBool>) -> Self {
        Self {
            ledger: MemoryAuditLedger::new(),
            armed,
        }
    }
}
impl AuditLedger for IntentAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let is_intent = event.phase() == AuditPhase::Intent
            && event.payload().as_value().get("file_operation").is_some();
        let record = self.ledger.append(event)?;
        if is_intent {
            self.armed.store(true, Ordering::SeqCst);
        }
        Ok(record)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}

pub(super) struct QueryAudit {
    ledger: Arc<dyn AuditLedger>,
    once: AtomicBool,
    pub(super) pause: Pause,
}
impl QueryAudit {
    pub(super) fn new(ledger: Arc<dyn AuditLedger>) -> Self {
        Self {
            ledger,
            once: AtomicBool::new(true),
            pause: Pause::default(),
        }
    }
}
impl AuditLedger for QueryAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        self.ledger.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        let records = self.ledger.query(filter)?;
        // 实际 SQLite 查询已经完成，但调用方尚未取得返回值。
        if self.once.swap(false, Ordering::SeqCst) {
            self.pause.enter();
        }
        Ok(records)
    }
}
