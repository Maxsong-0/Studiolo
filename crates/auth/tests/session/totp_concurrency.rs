//! 显式同步跨实例交错；不以 sleep 猜测验证码消费或审计进度。

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
fn same_code_concurrent_across_instances_is_consumed_and_published_only_once() {
    let h = TotpHarness::enrolled(3600, 7200);
    let issued = Arc::new(h.h.manager.issue(&identity()).unwrap());
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let (_, totp, _) = h.reopen();
        let issued = issued.clone();
        let barrier = barrier.clone();
        workers.push(thread::spawn(move || {
            let owner = identity();
            barrier.wait();
            totp.step_up(&request(&owner, &issued, 1), code(NEXT))
        }));
    }
    barrier.wait();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    for error in results.into_iter().filter_map(Result::err) {
        assert!(matches!(
            error,
            SessionError::Rejected | SessionError::OperationPending
        ));
    }
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 1);
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 0);
    assert_eq!(
        scalar(&h.h.manager, "SELECT last_step FROM totp_factor"),
        NEXT / 30_000
    );
    let (_, totp, _) = h.reopen();
    let owner = identity();
    assert_eq!(
        totp.step_up(&request(&owner, &issued, 2), code(NEXT))
            .unwrap_err(),
        SessionError::Rejected
    );
}

#[test]
fn pending_pins_revoked_session_until_delivery_and_late_activation_cannot_recreate_it() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let controller = h.h.manager.issue(&owner).unwrap();
    let target = Arc::new(h.h.manager.issue(&owner).unwrap());
    let (_, totp, _) = h.reopen();
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
    let target_for_worker = target.clone();
    let worker = thread::spawn(move || {
        let owner = identity();
        totp.step_up(&request(&owner, &target_for_worker, 1), code(NEXT))
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
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM sessions WHERE state='revoked'"
        ),
        1
    );
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 1);

    // 另一补记者完成精确双回执后才释放 pending；此时回收不能赋予权限。
    assert!(h.totp.flush_audit().unwrap() <= 2);
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 0);
    assert_eq!(
        h.h.manager.maintain_retention().unwrap().reclaimed_sessions,
        1
    );
    h.h.clock.set(NEXT + 30_000);
    let fresh = h
        .totp
        .step_up(&request(&owner, &controller, 1), code(NEXT + 30_000))
        .unwrap();
    resume_tx.send(()).unwrap();
    assert!(worker.join().unwrap().is_err());
    *h.h.audit.hook.lock().unwrap() = None;
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 1);
    h.totp
        .require_l3(
            &request(&owner, &controller, 1),
            SensitiveAction::RestoreVault,
        )
        .unwrap();
    assert_eq!(fresh.verified_at_ms, NEXT + 30_000);
    assert_eq!(
        h.h.manager.check(&owner, &target.token).unwrap_err(),
        SessionError::Rejected
    );
}
