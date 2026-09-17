use crate::{
    editing::refresh,
    file_operations::{delete, post},
    fixture::Fixture,
};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};

struct Failing {
    ledger: MemoryAuditLedger,
    failing: AtomicBool,
    phase: AuditPhase,
}
impl AuditLedger for Failing {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if self.failing.load(Ordering::Acquire)
            && event.phase() == self.phase
            && event.payload().as_value().get("file_operation").is_some()
        {
            return Err(AuditError::Unavailable);
        }
        self.ledger.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}

#[tokio::test]
async fn outcome_audit_failure_keeps_original_key_and_later_replay_is_historical() {
    let audit = Arc::new(Failing {
        ledger: MemoryAuditLedger::new(),
        failing: AtomicBool::new(true),
        phase: AuditPhase::Outcome,
    });
    let f = Fixture::file_operations(Some(audit.clone())).await;
    let issued = f.http.issue().await;
    let input = delete(&f, "unknown-outcome-001");
    assert_eq!(
        post(&f, &issued, &input, 503).await["code"],
        "outcome_unknown"
    );
    assert!(!f.http.root.join("vault/a.md").exists());
    assert!(!f.journal()["file_operation"].is_null());
    audit.failing.store(false, Ordering::Release);
    let saved = post(&f, &issued, &input, 200).await;
    fs::write(f.http.root.join("vault/a.md"), b"later external bytes").unwrap();
    refresh(&f, &issued).await;
    assert_eq!(post(&f, &issued, &input, 200).await, saved);
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        b"later external bytes"
    );
    assert_eq!(audit.query(&AuditFilter::default()).unwrap().len(), 2);
    f.close().await;
}

#[tokio::test]
async fn intent_failure_is_unknown_and_fixed_audit_flush_does_not_install_prepared() {
    let audit = Arc::new(Failing {
        ledger: MemoryAuditLedger::new(),
        failing: AtomicBool::new(true),
        phase: AuditPhase::Intent,
    });
    let f = Fixture::file_operations(Some(audit.clone())).await;
    let issued = f.http.issue().await;
    let input = delete(&f, "unknown-intent-001");
    assert_eq!(
        post(&f, &issued, &input, 503).await["code"],
        "outcome_unknown"
    );
    audit.failing.store(false, Ordering::Release);
    assert_eq!(
        post(&f, &issued, &input, 503).await["code"],
        "outcome_unknown"
    );
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        f.expected.content
    );
    assert_eq!(f.journal()["file_operation"]["phase"], "prepared");
    assert_eq!(audit.query(&AuditFilter::default()).unwrap().len(), 1);
    f.close().await;
}
