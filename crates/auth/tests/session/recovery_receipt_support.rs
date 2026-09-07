//! 故障 Adapter 只替换当前恢复事件；真实旧证据仍来自原 MemoryAudit Adapter。

use super::*;
use std::sync::Arc;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
    MemoryAuditLedger, SanitizedPayload,
};

#[derive(Clone, Copy)]
pub(super) enum Fault {
    Payload(AuditPhase),
    EventId(AuditPhase),
    ZeroOffset(AuditPhase),
    LowOutcome,
}

struct ReceiptAdapter {
    real: Arc<FaultAudit>,
    other: MemoryAuditLedger,
    fault: Fault,
}

impl AuditLedger for ReceiptAdapter {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let phase = match self.fault {
            Fault::Payload(phase) | Fault::EventId(phase) | Fault::ZeroOffset(phase) => phase,
            Fault::LowOutcome => AuditPhase::Outcome,
        };
        if event.action() != "totp.recovered" || event.phase() != phase {
            return self.real.append(event);
        }
        let replacement = match self.fault {
            Fault::Payload(_) => AuditEvent::new(
                event.event_id().clone(),
                event.phase(),
                event.action(),
                SanitizedPayload::try_from(
                    serde_json::json!({"unexpected":"missing-recovery-fields"}),
                )
                .unwrap(),
            ),
            Fault::EventId(_) => AuditEvent::new(
                AuditEventId::parse("recovery-receipt-foreign-event").unwrap(),
                event.phase(),
                event.action(),
                event.payload().clone(),
            ),
            Fault::ZeroOffset(_) | Fault::LowOutcome => event.clone(),
        };
        // 错事件仍有真实自洽哈希链；不能只靠 verify_integrity 当作来源或事件证明。
        let record = self.other.append(replacement)?;
        record.verify_integrity().unwrap();
        if matches!(self.fault, Fault::ZeroOffset(_)) {
            return Ok(AuditRecord::from_parts(
                0,
                record.recorded_at(),
                record.previous_hash(),
                record.record_hash(),
                event,
            ));
        }
        if matches!(self.fault, Fault::LowOutcome) {
            assert_eq!(record.event(), &event);
            assert_eq!(record.offset(), 1);
        }
        Ok(record)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.real.query(filter)
    }
}

pub(super) fn with_fault(h: &RecoveryHarness, fault: Fault) -> (Arc<SessionManager>, Recovery) {
    let audit = Arc::new(ReceiptAdapter {
        real: h.h.audit.clone(),
        other: MemoryAuditLedger::new(),
        fault,
    });
    let manager = Arc::new(
        SessionManager::open(
            &h.h.root,
            config(3600, 7200),
            audit,
            h.h.clock.clone(),
            h.h.random.clone(),
        )
        .unwrap(),
    );
    let recovery = Recovery::attach(manager.clone()).unwrap();
    (manager, recovery)
}

pub(super) fn assert_repaired_flush_still_cannot_switch(
    h: &RecoveryHarness,
    issued: &IssuedSession,
) {
    // 普通重开本身会完整验证格式和回执；错误 ACK 不得让 Auth 永久损坏。
    let (manager, recovery) = h.reopen();
    manager.check(&identity(), &issued.token).unwrap();
    assert!(recovery.flush_audit().unwrap() <= 2);
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM recovery_pending WHERE intent_receipt IS NOT NULL AND outcome_receipt IS NOT NULL"
        ),
        1
    );
    assert_eq!(
        scalar(
            &manager,
            "SELECT count(*) FROM recovery_codes WHERE slot=0 AND intent_receipt IS NOT NULL AND outcome_receipt IS NOT NULL"
        ),
        1
    );
    assert_eq!(
        scalar(&manager, "SELECT used_mask FROM recovery_control"),
        1
    );
    assert_eq!(
        scalar(&manager, "SELECT high_step FROM recovery_control"),
        CHANGE / 30_000
    );
    assert_eq!(
        scalar(&manager, "SELECT last_step FROM totp_factor"),
        NEXT / 30_000
    );
    assert!(Totp::attach(manager.clone(), seed(OLD)).is_ok());
    assert_eq!(
        Totp::attach(manager.clone(), seed(NEW)).err().unwrap(),
        SessionError::Configuration
    );
    h.h.clock.set(CHANGE + 300_000);
    recovery.flush_audit().unwrap();
    assert_eq!(scalar(&manager, "SELECT count(*) FROM recovery_pending"), 0);
    assert_eq!(
        scalar(&manager, "SELECT used_mask FROM recovery_control"),
        1
    );
    assert_eq!(
        scalar(&manager, "SELECT high_step FROM recovery_control"),
        CHANGE / 30_000
    );
    assert!(Totp::attach(manager, seed(OLD)).is_ok());
}
