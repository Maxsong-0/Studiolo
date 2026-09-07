//! 同一原始事件的另一个完整回执也不是原始回执，不能当成清理许可。

use super::signed_identity::stored_text;
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};

struct ChangedReceiptAudit {
    original: Arc<FaultAudit>,
    alternate: MemoryAuditLedger,
    change: AtomicBool,
}

impl AuditLedger for ChangedReceiptAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if self.change.load(Ordering::SeqCst)
            && event.action() == "session.issue"
            && event.phase() == AuditPhase::Outcome
        {
            // 同一事件的新账本回执本身完整，但偏移/前链不是原始权威回执。
            let changed = self.alternate.append(event.clone())?;
            changed.verify_integrity().unwrap();
            assert_ne!(changed, self.original.append(event)?);
            Ok(changed)
        } else {
            self.original.append(event)
        }
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.original.query(filter)
    }
}

#[test]
fn self_consistent_but_rewritten_original_receipt_never_authorizes_deletion() {
    let h = Harness::new(10, 100);
    let issued = h.manager.issue(&identity()).unwrap();
    h.manager
        .revoke_all(&identity(), &issued.token, &issued.csrf)
        .unwrap();
    let before = stored_text(
        &h.manager,
        "SELECT checkpoint_json FROM auth_retention_checkpoint",
    );
    let audit = Arc::new(ChangedReceiptAudit {
        original: h.audit.clone(),
        alternate: MemoryAuditLedger::new(),
        change: AtomicBool::new(true),
    });
    let manager = SessionManager::open(
        &h.root,
        config(10, 100),
        audit.clone(),
        h.clock.clone(),
        h.random.clone(),
    )
    .unwrap();
    assert_eq!(
        manager.maintain_retention().unwrap_err(),
        SessionError::CorruptStore
    );
    assert_eq!(scalar(&manager, "SELECT count(*) FROM sessions"), 1);
    assert_eq!(
        scalar(&manager, "SELECT count(*) FROM auth_retention_plan"),
        1
    );
    assert_eq!(
        stored_text(
            &manager,
            "SELECT checkpoint_json FROM auth_retention_checkpoint"
        ),
        before
    );
    assert!(
        audit
            .query(&AuditFilter::default().with_action("session.compacted"))
            .unwrap()
            .is_empty()
    );
    audit.change.store(false, Ordering::SeqCst);
    assert_eq!(manager.maintain_retention().unwrap().reclaimed_sessions, 1);
    assert_eq!(scalar(&manager, "SELECT count(*) FROM sessions"), 0);
}
