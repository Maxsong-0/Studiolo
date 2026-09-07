//! 多实例签发、审计阻塞和全部吊销的确定性交错。

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

#[test]
fn another_instance_open_never_abandons_a_live_issuer_and_revoke_all_wins() {
    let h = Harness::new(10, 100);
    let caller = h.manager.issue(&identity()).unwrap();
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let resume_rx = Mutex::new(resume_rx);
    let block = AtomicBool::new(true);
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.issue"
            && event.phase() == AuditPhase::Outcome
            && block.swap(false, Ordering::SeqCst)
        {
            entered_tx.send(()).unwrap();
            resume_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
    }));
    let manager = h.manager.clone();
    let worker = thread::spawn(move || manager.issue(&identity()));
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let other = h.reopen(10, 100);
    assert_eq!(
        scalar(
            &other,
            "SELECT count(*) FROM sessions WHERE state='pending'"
        ),
        1
    );
    assert_eq!(
        other
            .revoke_all(&identity(), &caller.token, &caller.csrf)
            .unwrap()
            .revoked,
        2
    );
    resume_tx.send(()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap_err(), SessionError::Rejected);
    assert_eq!(
        scalar(&other, "SELECT count(*) FROM sessions WHERE state='active'"),
        0
    );
}

#[test]
fn independent_instances_issue_and_check_without_shared_sqlite_connections() {
    let h = Harness::new(10, 100);
    let instances: Vec<_> = (0..4).map(|_| h.reopen(10, 100)).collect();
    let handles: Vec<_> = instances
        .into_iter()
        .map(|manager| {
            thread::spawn(move || {
                let issued = manager.issue(&identity()).unwrap();
                manager.check(&identity(), &issued.token).unwrap();
                issued.session.id
            })
        })
        .collect();
    let mut ids: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().unwrap().as_str().to_owned())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 4);
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        4
    );
}

#[test]
fn audit_callback_can_acquire_an_auth_write_transaction() {
    let h = Harness::new(10, 100);
    let root = h.root.clone();
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |_| {
        let connection = rusqlite::Connection::open(root.join("auth.sqlite3")).unwrap();
        connection.busy_timeout(Duration::from_millis(100)).unwrap();
        connection
            .execute_batch("BEGIN IMMEDIATE;ROLLBACK")
            .unwrap();
    }));
    h.manager.issue(&identity()).unwrap();
}
