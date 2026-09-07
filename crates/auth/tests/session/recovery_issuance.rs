//! 恢复推进 epoch 后，旧会话签发者即使持有迟到成功审计也不能发布凭据。

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
fn recovery_revokes_an_inflight_session_issue_and_late_issuer_cannot_reactivate_it() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (issued, codes) = h.ready();
    let prepared = h.prepared();
    let (manager, _) = h.reopen();
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let resume_rx = Mutex::new(resume_rx);
    let first = AtomicBool::new(true);
    let wait = Duration::from_secs(10);
    *h.h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.issue"
            && event.phase() == AuditPhase::Outcome
            && first.swap(false, Ordering::SeqCst)
        {
            entered_tx.send(()).unwrap();
            resume_rx.lock().unwrap().recv_timeout(wait).unwrap();
        }
    }));
    let worker = thread::spawn(move || manager.issue(&identity()));
    entered_rx.recv_timeout(wait).unwrap();
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM sessions WHERE state='pending'"
        ),
        1
    );
    let epoch = scalar(&h.h.manager, "SELECT epoch FROM auth_meta");
    h.h.clock.set(CHANGE);
    h.recovery
        .replace_factor(
            &request(&owner, &issued, 1),
            copy_code(&codes, 0),
            &prepared,
            otp(&NEW, CHANGE),
        )
        .unwrap();
    assert_eq!(
        scalar(&h.h.manager, "SELECT epoch FROM auth_meta"),
        epoch + 1
    );
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM sessions WHERE state='revoked'"
        ),
        2
    );
    resume_tx.send(()).unwrap();
    assert!(worker.join().unwrap().is_err());
    *h.h.audit.hook.lock().unwrap() = None;
    h.h.manager.flush_audit(256).unwrap();
    h.recovery.flush_audit().unwrap();
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM sessions WHERE state IN ('pending','active')"
        ),
        0
    );
    assert_eq!(
        h.h.manager.check(&owner, &issued.token).unwrap_err(),
        SessionError::Rejected
    );
    let fresh = h.h.manager.issue(&owner).unwrap();
    let (totp, _) = Totp::attach(h.h.manager.clone(), seed(NEW)).unwrap();
    assert_eq!(
        totp.require_l3(
            &request(&owner, &fresh, 1),
            SensitiveAction::ManageRecoveryCodes
        )
        .unwrap_err(),
        SessionError::ReauthenticationRequired
    );
}
