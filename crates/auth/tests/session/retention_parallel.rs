//! 两个维护者共用同一计划；已完成旧批次的迟到结果不能覆盖新检查点。

use super::signed_identity::stored_text;
use super::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

const WAIT: Duration = Duration::from_secs(10);

#[test]
fn two_maintainers_delete_once_and_late_completion_does_not_replace_new_checkpoint() {
    let h = Harness::new(10, 100);
    let owner = identity();
    let controller = h.manager.issue(&owner).unwrap();
    let target = h.manager.issue(&owner).unwrap();
    h.manager
        .revoke(
            &owner,
            &controller.token,
            &controller.csrf,
            &target.session.id,
        )
        .unwrap();
    let managers = [h.reopen(10, 100), h.reopen(10, 100)];
    let (entered_tx, entered_rx) = mpsc::sync_channel(2);
    let (early_tx, early_rx) = mpsc::sync_channel(1);
    let (late_tx, late_rx) = mpsc::sync_channel(1);
    let early_rx = Mutex::new(early_rx);
    let late_rx = Mutex::new(late_rx);
    let arrivals = AtomicUsize::new(0);
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.compacted" {
            let index = arrivals.fetch_add(1, Ordering::SeqCst);
            if index < 2 {
                entered_tx.send(index).unwrap();
                let resume = if index == 0 { &early_rx } else { &late_rx };
                resume.lock().unwrap().recv_timeout(WAIT).unwrap();
            }
        }
    }));
    let (finished_tx, finished_rx) = mpsc::sync_channel(2);
    let workers: Vec<_> = managers
        .into_iter()
        .map(|manager| {
            let finished = finished_tx.clone();
            thread::spawn(move || {
                let result = manager.maintain_retention();
                finished.send(result.clone()).unwrap();
                result
            })
        })
        .collect();
    let mut arrivals = [
        entered_rx.recv_timeout(WAIT).unwrap(),
        entered_rx.recv_timeout(WAIT).unwrap(),
    ];
    arrivals.sort();
    assert_eq!(arrivals, [0, 1]);
    assert_eq!(
        scalar(&h.manager, "SELECT count(*) FROM auth_retention_plan"),
        1
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 2);
    assert_eq!(
        h.manager.check(&owner, &controller.token).unwrap().state,
        SessionState::Active
    );

    early_tx.send(()).unwrap();
    assert_eq!(
        finished_rx
            .recv_timeout(WAIT)
            .unwrap()
            .unwrap()
            .reclaimed_sessions,
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
    let newest = stored_text(
        &h.manager,
        "SELECT checkpoint_json FROM auth_retention_checkpoint",
    );
    late_tx.send(()).unwrap();
    assert_eq!(
        finished_rx
            .recv_timeout(WAIT)
            .unwrap()
            .unwrap()
            .reclaimed_sessions,
        0
    );
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    *h.audit.hook.lock().unwrap() = None;
    assert_eq!(
        stored_text(
            &h.manager,
            "SELECT checkpoint_json FROM auth_retention_checkpoint"
        ),
        newest
    );
    assert_eq!(
        scalar(&h.manager, "SELECT count(*) FROM auth_retention_plan"),
        0
    );
    assert_eq!(
        scalar(&h.manager, "SELECT count(*) FROM auth_retention_checkpoint"),
        1
    );
    assert_eq!(
        h.manager.sessions(&owner, &controller.token).unwrap(),
        vec![controller.session]
    );
    assert_eq!(
        h.audit
            .query(&AuditFilter::default().with_action("session.compacted"))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn concurrent_issue_and_revoke_all_do_not_expand_frozen_deletion_candidates() {
    let h = Harness::new(10, 100);
    let owner = identity();
    let controller = h.manager.issue(&owner).unwrap();
    let target = h.manager.issue(&owner).unwrap();
    h.manager
        .revoke(
            &owner,
            &controller.token,
            &controller.csrf,
            &target.session.id,
        )
        .unwrap();
    let other = h.reopen(10, 100);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let once = AtomicBool::new(true);
    let resume_rx = Mutex::new(resume_rx);
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.compacted" && once.swap(false, Ordering::SeqCst) {
            entered_tx.send(()).unwrap();
            resume_rx.lock().unwrap().recv_timeout(WAIT).unwrap();
        }
    }));
    let worker = thread::spawn(move || other.maintain_retention());
    entered_rx.recv_timeout(WAIT).unwrap();
    let newer = h.manager.issue(&owner).unwrap();
    assert_eq!(
        h.manager
            .revoke_all(&owner, &controller.token, &controller.csrf)
            .unwrap()
            .revoked,
        2
    );
    resume_tx.send(()).unwrap();
    let report = worker.join().unwrap().unwrap();
    *h.audit.hook.lock().unwrap() = None;
    assert_eq!(report.reclaimed_sessions, 1);
    assert_eq!(report.remaining, RetentionWork::More);
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 2);
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='revoked'"
        ),
        2
    );
    assert_eq!(
        h.manager.maintain_retention().unwrap().reclaimed_sessions,
        2
    );
    assert_eq!(
        h.manager.check(&owner, &newer.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        h.manager.check(&owner, &controller.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 0);
}
