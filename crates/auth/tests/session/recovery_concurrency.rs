//! 显式同步真实双实例；晚到回执与原调用不能重建已回收会话或发布因子。

use super::*;
use std::{
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use studiolo_audit_ledger::AuditPhase;

const WAIT: Duration = Duration::from_secs(10);

#[test]
fn same_recovery_code_across_instances_switches_exactly_once() {
    let h = RecoveryHarness::new(3600, 7200);
    let (issued, codes) = h.ready();
    let issued = Arc::new(issued);
    h.h.clock.set(CHANGE);
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let (manager, recovery) = h.reopen();
        let prepared = PreparedTotpFactor::prepare(manager, seed(NEW)).unwrap();
        let code = copy_code(&codes, 0);
        let issued = issued.clone();
        let barrier = barrier.clone();
        workers.push(thread::spawn(move || {
            let owner = identity();
            barrier.wait();
            recovery.replace_factor(
                &request(&owner, &issued, 1),
                code,
                &prepared,
                otp(&NEW, CHANGE),
            )
        }));
    }
    barrier.wait();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        0
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_codes"),
        0
    );
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        0
    );
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 0);
    assert!(Totp::attach(h.h.manager.clone(), seed(NEW)).is_ok());
}

#[test]
fn revoked_pending_pins_session_until_exact_delivery_and_late_ack_cannot_switch() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (target, codes) = h.ready();
    let target = Arc::new(target);
    let controller = h.h.manager.issue(&owner).unwrap();
    let (manager, recovery) = h.reopen();
    let prepared = PreparedTotpFactor::prepare(manager, seed(NEW)).unwrap();
    let code = copy_code(&codes, 0);
    h.h.clock.set(CHANGE);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let resume_rx = Mutex::new(resume_rx);
    let first = AtomicBool::new(true);
    *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "totp.recovered"
            && event.phase() == AuditPhase::Outcome
            && first.swap(false, Ordering::SeqCst)
        {
            entered_tx.send(()).unwrap();
            resume_rx.lock().unwrap().recv_timeout(WAIT).unwrap();
        }
    }));
    let moving = target.clone();
    let worker = thread::spawn(move || {
        let owner = identity();
        recovery.replace_factor(
            &request(&owner, &moving, 1),
            code,
            &prepared,
            otp(&NEW, CHANGE),
        )
    });
    entered_rx.recv_timeout(WAIT).unwrap();
    h.h.manager
        .revoke(
            &owner,
            &controller.token,
            &controller.csrf,
            &target.session.id,
        )
        .unwrap();
    assert_eq!(
        h.h.manager.maintain_retention().unwrap().reclaimed_sessions,
        0
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        1
    );
    assert!(h.recovery.flush_audit().unwrap() <= 2);
    assert_eq!(
        scalar(&h.h.manager, "SELECT count(*) FROM recovery_pending"),
        0
    );
    assert_eq!(
        h.h.manager.maintain_retention().unwrap().reclaimed_sessions,
        1
    );
    h.h.clock.set(CHANGE + 30_000);
    h.totp
        .step_up(&request(&owner, &controller, 2), otp(&OLD, CHANGE + 30_000))
        .unwrap();
    resume_tx.send(()).unwrap();
    assert!(worker.join().unwrap().is_err());
    *h.h.audit.hook.lock().unwrap() = None;
    assert!(Totp::attach(h.h.manager.clone(), seed(OLD)).is_ok());
    assert_eq!(
        Totp::attach(h.h.manager.clone(), seed(NEW)).err().unwrap(),
        SessionError::Configuration
    );
    h.totp
        .require_l3(
            &request(&owner, &controller, 2),
            SensitiveAction::ManageRecoveryCodes,
        )
        .unwrap();
    assert_eq!(
        h.h.manager.check(&owner, &target.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT used_mask FROM recovery_control"),
        1
    );
}
