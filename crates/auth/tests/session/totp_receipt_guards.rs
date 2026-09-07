//! 自洽但不可信的审计回执，以及已被新决定替代后的迟到确认。

use super::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
    MemoryAuditLedger, SanitizedPayload,
};

enum Fault {
    Payload(AuditPhase),
    LowOutcome,
    History(String),
}

struct ReceiptAdapter {
    real: Arc<FaultAudit>,
    other: MemoryAuditLedger,
    fault: Fault,
}

impl ReceiptAdapter {
    fn new(real: Arc<FaultAudit>, fault: Fault) -> Self {
        let other = MemoryAuditLedger::new();
        if matches!(&fault, Fault::History(_)) {
            let payload =
                SanitizedPayload::try_from(serde_json::json!({"fixture":"foreign-prefix"}))
                    .unwrap();
            other
                .append(AuditEvent::new(
                    AuditEventId::parse("receipt-guard-prefix").unwrap(),
                    AuditPhase::Outcome,
                    "fixture.prefix",
                    payload,
                ))
                .unwrap();
        }
        Self { real, other, fault }
    }
}

impl AuditLedger for ReceiptAdapter {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let faulty = match &self.fault {
            Fault::Payload(phase) => event.action() == "totp.step_up" && event.phase() == *phase,
            Fault::LowOutcome => {
                event.action() == "totp.step_up" && event.phase() == AuditPhase::Outcome
            }
            Fault::History(id) => event.event_id().as_str() == id,
        };
        if !faulty {
            return self.real.append(event);
        }
        let replacement = if matches!(&self.fault, Fault::Payload(_)) {
            let mut payload = event.payload().as_value().clone();
            payload["decision"] = serde_json::json!("different-decision");
            AuditEvent::new(
                event.event_id().clone(),
                event.phase(),
                event.action(),
                SanitizedPayload::try_from(payload).unwrap(),
            )
        } else {
            event.clone()
        };
        // 使用真实追加生成完整哈希链，不手造损坏 record，也不污染权威 Ledger。
        let record = self.other.append(replacement)?;
        record.verify_integrity().unwrap();
        if matches!(&self.fault, Fault::LowOutcome) {
            assert_eq!(record.event(), &event);
            assert_eq!(record.offset(), 1);
        }
        Ok(record)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.real.query(filter)
    }
}

fn with_fault(h: &TotpHarness, fault: Fault) -> (Arc<SessionManager>, Totp) {
    let audit = Arc::new(ReceiptAdapter::new(h.h.audit.clone(), fault));
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
    let (totp, _) = Totp::attach(manager.clone(), seed()).unwrap();
    (manager, totp)
}

fn grants(manager: &SessionManager) -> Vec<String> {
    text_rows(
        manager,
        "SELECT operation_json FROM totp_grants ORDER BY session_id",
    )
}

fn pending(manager: &SessionManager, condition: &str) -> i64 {
    scalar(
        manager,
        &format!("SELECT count(*) FROM totp_pending WHERE {condition}"),
    )
}

fn step(
    totp: &Totp,
    issued: &IssuedSession,
    now: i64,
) -> Result<RecentAuthentication, SessionError> {
    totp.step_up(&request(&identity(), issued, 1), code(now))
}

fn require(totp: &Totp, issued: &IssuedSession) -> Result<(), SessionError> {
    totp.require_l3(
        &request(&identity(), issued, 1),
        SensitiveAction::ChangeVault,
    )
}

fn recover(h: &TotpHarness, issued: &IssuedSession, failed_at: i64, previous: &[String]) {
    // 正常重开就是完整格式/回执验证；坏回执不能令整个 AuthDB 永久不可打开。
    let (manager, totp, _) = h.reopen();
    assert!(manager.check(&identity(), &issued.token).is_ok());
    assert_eq!(pending(&manager, "outcome_receipt IS NULL"), 1);
    assert!(totp.flush_audit().unwrap() <= 2);
    assert_eq!(grants(&manager).as_slice(), previous);
    assert_eq!(pending(&manager, "outcome_receipt IS NOT NULL"), 1);
    h.h.clock.set(failed_at + 300_000);
    totp.flush_audit().unwrap();
    assert_eq!(pending(&manager, "1"), 0);
    assert_eq!(
        require(&totp, issued).unwrap_err(),
        SessionError::ReauthenticationRequired
    );
    step(&totp, issued, failed_at + 300_000).unwrap();
    require(&totp, issued).unwrap();
}

#[test]
fn genuine_wrong_payload_receipts_are_not_acknowledged_or_recovered_as_l3() {
    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
        let h = TotpHarness::enrolled(3600, 7200);
        let owner = identity();
        let issued = h.h.manager.issue(&owner).unwrap();
        let (manager, totp) = with_fault(&h, Fault::Payload(phase));
        assert_eq!(
            step(&totp, &issued, NEXT).unwrap_err(),
            SessionError::AuditUnavailable
        );
        let acknowledged = pending(&manager, "intent_receipt IS NOT NULL");
        assert_eq!(acknowledged, i64::from(phase == AuditPhase::Outcome));
        assert!(grants(&manager).is_empty());
        recover(&h, &issued, NEXT, &[]);
    }
}

#[test]
fn matching_outcome_from_empty_ledger_cannot_precede_the_acknowledged_intent() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    let (manager, totp) = with_fault(&h, Fault::LowOutcome);
    assert_eq!(
        step(&totp, &issued, NEXT).unwrap_err(),
        SessionError::AuditUnavailable
    );
    let encoded = text_rows(&manager, "SELECT intent_receipt FROM totp_pending");
    let intent: AuditRecord = serde_json::from_str(&encoded[0]).unwrap();
    assert!(intent.offset() > 1);
    assert!(grants(&manager).is_empty());
    recover(&h, &issued, NEXT, &[]);
}

#[test]
fn replacing_enrollment_or_old_grant_with_another_ledger_history_is_rejected() {
    for old_grant in [false, true] {
        let h = TotpHarness::enrolled(3600, 7200);
        let owner = identity();
        let issued = h.h.manager.issue(&owner).unwrap();
        if old_grant {
            step(&h.totp, &issued, NEXT).unwrap();
        }
        let previous = grants(&h.h.manager);
        let action = if old_grant {
            "totp.step_up"
        } else {
            "totp.enrolled"
        };
        let originals =
            h.h.audit
                .query(&AuditFilter::default().with_action(action))
                .unwrap();
        let event_id = originals[0].event().event_id().as_str().to_owned();
        let failed_at = if old_grant { NEXT + 30_000 } else { NEXT };
        h.h.clock.set(failed_at);
        let (manager, totp) = with_fault(&h, Fault::History(event_id));
        assert_eq!(
            step(&totp, &issued, failed_at).unwrap_err(),
            SessionError::AuditUnavailable
        );
        assert_eq!(pending(&manager, "intent_receipt IS NULL"), 1);
        assert_eq!(pending(&manager, "outcome_receipt IS NULL"), 1);
        assert_eq!(grants(&manager), previous);
        recover(&h, &issued, failed_at, &previous);
    }
}

#[test]
fn late_outcome_ack_cannot_replace_a_new_grant_for_the_same_live_session() {
    const WAIT: Duration = Duration::from_secs(10);
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    h.h.audit.fail_outcome.store(true, Ordering::SeqCst);
    assert!(step(&h.totp, &issued, NEXT).is_err());
    h.h.audit.fail_outcome.store(false, Ordering::SeqCst);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let resume_rx = Mutex::new(resume_rx);
    let first = AtomicBool::new(true);
    *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "totp.step_up"
            && event.phase() == AuditPhase::Outcome
            && first.swap(false, Ordering::SeqCst)
        {
            entered_tx.send(()).unwrap();
            resume_rx.lock().unwrap().recv_timeout(WAIT).unwrap();
        }
    }));
    let (_, delayed, _) = h.reopen();
    let worker = thread::spawn(move || delayed.flush_audit());
    entered_rx.recv_timeout(WAIT).unwrap();
    h.totp.flush_audit().unwrap();
    assert!(grants(&h.h.manager).is_empty());
    h.h.clock.set(NEXT + 300_000);
    h.totp.flush_audit().unwrap();
    step(&h.totp, &issued, NEXT + 300_000).unwrap();
    let current = grants(&h.h.manager);
    resume_tx.send(()).unwrap();
    worker.join().unwrap().unwrap();
    *h.h.audit.hook.lock().unwrap() = None;
    assert_eq!(grants(&h.h.manager), current);
    assert_eq!(current.len(), 1);
    assert_eq!(pending(&h.h.manager, "1"), 0);
    require(&h.totp, &issued).unwrap();
}
