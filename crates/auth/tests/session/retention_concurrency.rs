//! 有序通道强制迟到签发和补记；等待时不持 Auth 锁，也不以 sleep 猜测交错。

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
use studiolo_audit_ledger::{AuditEvent, AuditPhase};

const WAIT: Duration = Duration::from_secs(10);

fn pause_once(
    audit: &FaultAudit,
    predicate: impl Fn(&AuditEvent) -> bool + Send + Sync + 'static,
) -> (mpsc::Receiver<()>, mpsc::SyncSender<()>) {
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let first = AtomicBool::new(true);
    let resume_rx = Mutex::new(resume_rx);
    *audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if predicate(event) && first.swap(false, Ordering::SeqCst) {
            entered_tx.send(()).unwrap();
            resume_rx.lock().unwrap().recv_timeout(WAIT).unwrap();
        }
    }));
    (entered_rx, resume_tx)
}

#[test]
fn acknowledged_pending_is_not_collected_or_activated_by_another_maintainer() {
    let h = Harness::new(10, 100);
    let owner = identity();
    let controller = h.manager.issue(&owner).unwrap();
    let old = h.manager.issue(&owner).unwrap();
    h.manager
        .revoke(&owner, &controller.token, &controller.csrf, &old.session.id)
        .unwrap();
    let other = h.reopen(10, 100);
    let (entered, resume) = pause_once(&h.audit, |event| {
        event.action() == "session.issue" && event.phase() == AuditPhase::Outcome
    });
    let worker = thread::spawn(move || other.issue(&identity()));
    entered.recv_timeout(WAIT).unwrap();
    let pending = h
        .manager
        .sessions(&owner, &controller.token)
        .unwrap()
        .into_iter()
        .find(|row| row.state == SessionState::Pending)
        .unwrap();
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='pending' AND grant_receipt IS NULL"
        ),
        1
    );

    // 维护可补记该 pending 的原事件，但只能回收另一个已终态会话。
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        1
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='pending' AND grant_receipt IS NOT NULL"
        ),
        1
    );
    let rows = h.manager.sessions(&owner, &controller.token).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.contains(&pending));
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        1
    );
    resume.send(()).unwrap();
    let issued = worker.join().unwrap().unwrap();
    *h.audit.hook.lock().unwrap() = None;
    assert_eq!(issued.session.id, pending.id);
    assert_eq!(
        h.manager.check(&owner, &issued.token).unwrap().state,
        SessionState::Active
    );
}

#[test]
fn late_issuer_cannot_recreate_collected_pending_after_revoke_all() {
    let h = Harness::new(10, 100);
    let owner = identity();
    let controller = h.manager.issue(&owner).unwrap();
    let other = h.reopen(10, 100);
    let (entered, resume) = pause_once(&h.audit, |event| {
        event.action() == "session.issue" && event.phase() == AuditPhase::Outcome
    });
    let worker = thread::spawn(move || other.issue(&identity()));
    entered.recv_timeout(WAIT).unwrap();
    let pending = h
        .manager
        .sessions(&owner, &controller.token)
        .unwrap()
        .into_iter()
        .find(|row| row.state == SessionState::Pending)
        .unwrap();
    assert_eq!(
        h.manager
            .revoke_all(&owner, &controller.token, &controller.csrf)
            .unwrap()
            .revoked,
        2
    );
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        2
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 0);
    let fresh = h.manager.issue(&owner).unwrap();
    assert_ne!(fresh.session.id, pending.id);
    resume.send(()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap_err(), SessionError::Rejected);
    *h.audit.hook.lock().unwrap() = None;
    assert_eq!(
        h.manager.sessions(&owner, &fresh.token).unwrap(),
        vec![fresh.session]
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='pending'"
        ),
        0
    );
    assert_eq!(
        h.manager.check(&owner, &controller.token).unwrap_err(),
        SessionError::Rejected
    );
}

#[test]
fn late_ack_of_collected_event_cannot_recreate_row_or_regress_new_checkpoint() {
    let h = Harness::new(10, 100);
    let owner = identity();
    let controller = h.manager.issue(&owner).unwrap();
    let target = h.manager.issue(&owner).unwrap();
    h.audit.fail.store(true, Ordering::SeqCst);
    h.manager
        .revoke(
            &owner,
            &controller.token,
            &controller.csrf,
            &target.session.id,
        )
        .unwrap();
    h.audit.fail.store(false, Ordering::SeqCst);
    let other = h.reopen(10, 100);
    let target_id = target.session.id.as_str().to_owned();
    let (entered, resume) = pause_once(&h.audit, move |event| {
        event.action() == "session.revoked"
            && event.phase() == AuditPhase::Intent
            && event.payload().as_value()["target"].as_str() == Some(target_id.as_str())
    });
    let worker = thread::spawn(move || other.flush_audit(1));
    entered.recv_timeout(WAIT).unwrap();
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        1
    );
    let newer = h.manager.issue(&owner).unwrap();
    h.manager
        .revoke(
            &owner,
            &controller.token,
            &controller.csrf,
            &newer.session.id,
        )
        .unwrap();
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        1
    );
    let checkpoint = super::signed_identity::stored_text(
        &h.manager,
        "SELECT checkpoint_json FROM auth_retention_checkpoint",
    );
    let events = scalar(&h.manager, "SELECT count(*) FROM audit_outbox");
    resume.send(()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap(), 1);
    *h.audit.hook.lock().unwrap() = None;
    assert_eq!(
        super::signed_identity::stored_text(
            &h.manager,
            "SELECT checkpoint_json FROM auth_retention_checkpoint",
        ),
        checkpoint
    );
    assert_eq!(
        scalar(&h.manager, "SELECT count(*) FROM audit_outbox"),
        events
    );
    assert_eq!(
        h.manager.sessions(&owner, &controller.token).unwrap(),
        vec![controller.session]
    );
    assert_eq!(
        h.manager.check(&owner, &target.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        h.manager.check(&owner, &newer.token).unwrap_err(),
        SessionError::Rejected
    );
}
