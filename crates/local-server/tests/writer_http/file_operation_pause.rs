use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};

#[derive(Default)]
pub(super) struct Paused {
    pub(super) ledger: MemoryAuditLedger,
    state: Mutex<(bool, bool)>,
    gate: Condvar,
}
impl Paused {
    pub(super) fn wait(&self) {
        let guard = self.state.lock().unwrap();
        let waited = self
            .gate
            .wait_timeout_while(guard, Duration::from_secs(10), |state| !state.0)
            .unwrap();
        assert!(!waited.1.timed_out());
    }
    pub(super) fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.gate.notify_all();
    }
}
impl AuditLedger for Paused {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.phase() == AuditPhase::Intent
            && event.payload().as_value().get("file_operation").is_some()
        {
            let mut state = self.state.lock().unwrap();
            state.0 = true;
            self.gate.notify_all();
            let waited = self
                .gate
                .wait_timeout_while(state, Duration::from_secs(10), |state| !state.1)
                .unwrap();
            assert!(!waited.1.timed_out());
        }
        self.ledger.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}
