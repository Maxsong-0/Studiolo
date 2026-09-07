//! 只读审计替换 Adapter；坏响应均由真实账本产生或明确模拟查询故障。

use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
    MemoryAuditLedger,
};

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub(super) enum Fault {
    None,
    Query,
    MissingIntent,
    MissingOutcome,
    Changed,
    Duplicate,
}

type Hook = Arc<dyn Fn() + Send + Sync>;

pub(super) struct ReadAudit {
    source: Arc<dyn AuditLedger>,
    pub fault: AtomicU8,
    pub appends: AtomicUsize,
    pub queries: AtomicUsize,
    pub hook: Mutex<Option<Hook>>,
}

impl ReadAudit {
    pub fn new(source: Arc<dyn AuditLedger>) -> Self {
        Self {
            source,
            fault: AtomicU8::new(Fault::None as u8),
            appends: AtomicUsize::new(0),
            queries: AtomicUsize::new(0),
            hook: Mutex::new(None),
        }
    }
}

impl AuditLedger for ReadAudit {
    fn append(&self, _: AuditEvent) -> Result<AuditRecord, AuditError> {
        self.appends.fetch_add(1, Ordering::SeqCst);
        Err(AuditError::Unavailable)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        assert!(filter.event_id().is_some());
        assert!(filter.phase().is_some());
        assert_eq!(filter.limit(), Some(2));
        let hook = self.hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
        let fault = self.fault.load(Ordering::SeqCst);
        if fault == Fault::Query as u8 {
            return Err(AuditError::Unavailable);
        }
        if (fault == Fault::MissingIntent as u8 && filter.phase() == Some(AuditPhase::Intent))
            || (fault == Fault::MissingOutcome as u8 && filter.phase() == Some(AuditPhase::Outcome))
        {
            return Ok(Vec::new());
        }
        let mut records = self.source.query(filter)?;
        if let Some(record) = records.first() {
            if fault == Fault::Changed as u8 {
                let other = MemoryAuditLedger::default();
                for id in ["readiness-prefix-a", "readiness-prefix-b"] {
                    other.append(AuditEvent::new(
                        AuditEventId::parse(id).unwrap(),
                        AuditPhase::Intent,
                        "fixture",
                        record.event().payload().clone(),
                    ))?;
                }
                // 自洽且事件相同，但 offset=3 不属于原登记的 1/2 历史。
                return Ok(vec![other.append(record.event().clone())?]);
            }
            if fault == Fault::Duplicate as u8 {
                records.push(record.clone());
            }
        }
        Ok(records)
    }
}

pub(super) fn open(h: &Harness, audit: Arc<dyn AuditLedger>) -> SessionManager {
    SessionManager::open_existing(
        &h.root,
        config(10, 100),
        audit,
        h.clock.clone(),
        h.random.clone(),
    )
    .unwrap()
}
