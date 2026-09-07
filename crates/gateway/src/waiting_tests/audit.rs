//! 首调用停在 Intent；释放结果由测试选择，拒绝审计可单独失败。

use super::support::{TIMEOUT, assert_slots_unlocked};
use crate::*;
use std::sync::{
    Weak,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
use studiolo_audit_ledger::{AuditFilter, AuditRecord, MemoryAuditLedger};

pub(super) struct GatedAudit {
    inner: MemoryAuditLedger,
    first: AtomicBool,
    entered: Mutex<mpsc::Receiver<()>>,
    signal: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<bool>>,
    resume: mpsc::Sender<bool>,
    pub(super) denial_failure: AtomicBool,
    pub(super) denial_conflict: AtomicBool,
    pub(super) intents: AtomicUsize,
    pub(super) gateway: Mutex<Weak<MemoryGateway>>,
    denials: Mutex<Vec<AuditEvent>>,
}

impl GatedAudit {
    pub(super) fn new() -> Self {
        let (signal, entered) = mpsc::channel();
        let (resume, release) = mpsc::channel();
        Self {
            inner: MemoryAuditLedger::new(),
            first: AtomicBool::new(true),
            signal,
            entered: Mutex::new(entered),
            resume,
            release: Mutex::new(release),
            denial_failure: AtomicBool::new(false),
            denial_conflict: AtomicBool::new(false),
            intents: AtomicUsize::new(0),
            gateway: Mutex::new(Weak::new()),
            denials: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn wait_first(&self) {
        self.entered
            .lock()
            .unwrap()
            .recv_timeout(TIMEOUT)
            .expect("first call did not reach Intent");
    }

    pub(super) fn release_first(&self, succeed: bool) {
        self.resume.send(succeed).unwrap();
    }

    pub(super) fn cancel_first(&self) {
        // 测试在等待断言处失败也释放首调用，不让其审计夹具永久占位。
        let _ = self.resume.send(false);
    }

    pub(super) fn assert_denial(&self, expected: &GatewayError, failed: bool) {
        let denials = self.denials.lock().unwrap();
        assert_eq!(denials.len(), 1);
        let event = &denials[0];
        assert_eq!(event.phase(), AuditPhase::Outcome);
        assert_eq!(
            event.payload().as_value()["reason"],
            expected.audit_reason()
        );
        assert!(
            event
                .event_id()
                .to_string()
                .ends_with(&format!(":deny:{}", expected.audit_reason()))
        );
        let records = self
            .inner
            .query(&AuditFilter::default().with_event_id(event.event_id().clone()))
            .unwrap();
        assert_eq!(records.len(), usize::from(!failed));
    }
}

impl AuditLedger for GatedAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        assert_slots_unlocked(&self.gateway);
        if event.phase() == AuditPhase::Intent {
            self.intents.fetch_add(1, Ordering::SeqCst);
            if self.first.swap(false, Ordering::SeqCst) {
                self.signal.send(()).unwrap();
                if !self
                    .release
                    .lock()
                    .unwrap()
                    .recv_timeout(TIMEOUT)
                    .expect("first call was not released")
                {
                    return Err(AuditError::Unavailable);
                }
            }
        }
        if event.payload().as_value()["decision"] == "deny" {
            self.denials.lock().unwrap().push(event.clone());
            if self.denial_failure.load(Ordering::SeqCst) {
                return Err(AuditError::Unavailable);
            }
            if self.denial_conflict.load(Ordering::SeqCst) {
                return Err(AuditError::IdempotencyConflict {
                    event_id: event.event_id().clone(),
                    phase: event.phase(),
                });
            }
        }
        self.inner.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}
