//! 覆盖尚未落 pending 的旧签发，以及已过绝对期限的 pending；均不能靠迟到完成复活。

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
use studiolo_audit_ledger::AuditPhase;

const WAIT: Duration = Duration::from_secs(10);

struct PausedIssue {
    worker: thread::JoinHandle<Result<IssuedSession, SessionError>>,
    entered: mpsc::Receiver<()>,
    resume: mpsc::SyncSender<()>,
}

fn paused_issue(h: &Harness, phase: AuditPhase) -> PausedIssue {
    let other = h.reopen(10, 100);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let once = AtomicBool::new(true);
    let resume_rx = Mutex::new(resume_rx);
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.issue"
            && event.phase() == phase
            && once.swap(false, Ordering::SeqCst)
        {
            entered_tx.send(()).unwrap();
            resume_rx.lock().unwrap().recv_timeout(WAIT).unwrap();
        }
    }));
    let worker = thread::spawn(move || other.issue(&identity()));
    PausedIssue {
        worker,
        entered: entered_rx,
        resume: resume_tx,
    }
}

#[test]
fn reserved_issuer_before_pending_cannot_insert_behind_completed_retention_floor() {
    let h = Harness::new(10, 100);
    let owner = identity();
    let controller = h.manager.issue(&owner).unwrap();
    let PausedIssue {
        worker,
        entered,
        resume,
    } = paused_issue(&h, AuditPhase::Intent);
    entered.recv_timeout(WAIT).unwrap();
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 1);
    // 使用单目标吊销以保持 epoch 不变，单独检验签发序号地板而非 revoke_all 屏障。
    h.manager
        .revoke(
            &owner,
            &controller.token,
            &controller.csrf,
            &controller.session.id,
        )
        .unwrap();
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        1
    );
    assert_eq!(
        scalar(&h.manager, "SELECT epoch FROM auth_meta WHERE id=1"),
        1
    );
    let fresh = h.manager.issue(&owner).unwrap();
    resume.send(()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap_err(), SessionError::Rejected);
    *h.audit.hook.lock().unwrap() = None;
    assert_eq!(
        h.manager.sessions(&owner, &fresh.token).unwrap(),
        vec![fresh.session]
    );
    assert_eq!(
        scalar(&h.manager, "SELECT total FROM auth_denials WHERE kind=6"),
        1
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='pending'"
        ),
        0
    );
}

#[test]
fn expired_active_and_abandoned_pending_are_collected_without_late_activation() {
    let h = Harness::new(10, 100);
    let owner = identity();
    let controller = h.manager.issue(&owner).unwrap();
    let PausedIssue {
        worker,
        entered,
        resume,
    } = paused_issue(&h, AuditPhase::Outcome);
    entered.recv_timeout(WAIT).unwrap();
    h.clock.set(1_100_000);
    assert_eq!(
        h.manager.check(&owner, &controller.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='abandoned'"
        ),
        1
    );
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        2
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 0);
    resume.send(()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap_err(), SessionError::Rejected);
    *h.audit.hook.lock().unwrap() = None;
    let reopened = h.reopen(10, 100);
    assert_eq!(scalar(&reopened, "SELECT count(*) FROM sessions"), 0);
    let fresh = reopened.issue(&owner).unwrap();
    assert_eq!(
        reopened.check(&owner, &fresh.token).unwrap().state,
        SessionState::Active
    );
    assert_ne!(fresh.session.id, controller.session.id);
}
