//! 消费时间是 L3 窗口起点；容量拒绝也必须提交已观察时间及安全方向失效。

use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicI64, Ordering},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

struct AdvancingClock {
    now: AtomicI64,
    advance_after_read: AtomicBool,
}

impl SessionClock for AdvancingClock {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        let now = self.now.load(Ordering::SeqCst);
        if self.advance_after_read.swap(false, Ordering::SeqCst) {
            self.now.store(now + 1000, Ordering::SeqCst);
        }
        Ok(now)
    }
}

#[test]
fn window_starts_at_consumption_even_when_clock_advances_after_budget_admission() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    let clock = Arc::new(AdvancingClock {
        now: AtomicI64::new(NEXT),
        advance_after_read: AtomicBool::new(false),
    });
    let manager = Arc::new(
        SessionManager::open(
            &h.h.root,
            config(3600, 7200),
            h.h.audit.clone(),
            clock.clone(),
            h.h.random.clone(),
        )
        .unwrap(),
    );
    let (totp, _) = Totp::attach(manager, seed()).unwrap();
    // 只在此调用的首次时间读取后推进，不依赖打开/迁移内部调用了多少次时钟。
    clock.advance_after_read.store(true, Ordering::SeqCst);
    let proof = totp
        .step_up(&request(&owner, &issued, 1), code(NEXT + 1000))
        .unwrap();
    assert_eq!(proof.verified_at_ms, NEXT + 1000);
    assert_eq!(proof.expires_at_ms, NEXT + 301_000);
    clock.now.store(proof.expires_at_ms - 1, Ordering::SeqCst);
    totp.require_l3(&request(&owner, &issued, 1), SensitiveAction::ChangeSync)
        .unwrap();
    clock.now.store(proof.expires_at_ms, Ordering::SeqCst);
    assert_eq!(
        totp.require_l3(&request(&owner, &issued, 1), SensitiveAction::ChangeSync)
            .unwrap_err(),
        SessionError::ReauthenticationRequired
    );
}

#[test]
fn exhausted_operation_counter_commits_floor_and_expiry_without_consuming_or_elevating() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let short = crate::access::test_identity_with("fictional-person", 1065);
    let victim = h.h.manager.issue(&short).unwrap();
    let controller = h.h.manager.issue(&owner).unwrap();
    h.totp
        .step_up(&request(&short, &victim, 1), code(NEXT))
        .unwrap();
    let before =
        h.h.audit
            .query(&AuditFilter::default().with_action("totp.step_up"))
            .unwrap();
    let denials = scalar(&h.h.manager, "SELECT sum(total) FROM auth_denials");
    h.h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE totp_control SET next_operation=?1", [i64::MAX])
        .unwrap();
    h.h.clock.set(NEXT + 30_000);
    assert_eq!(
        h.totp
            .step_up(&request(&owner, &controller, 1), code(NEXT + 30_000))
            .unwrap_err(),
        SessionError::Capacity
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT clock_floor FROM auth_meta"),
        NEXT + 30_000
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT sum(total) FROM auth_denials"),
        denials + 1
    );
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT last_step FROM totp_factor"),
        NEXT / 30_000
    );
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 0);
    assert_eq!(
        h.h.audit
            .query(&AuditFilter::default().with_action("totp.step_up"))
            .unwrap(),
        before
    );
    h.h.clock.set(NEXT);
    assert_eq!(
        h.totp
            .require_l3(&request(&owner, &victim, 1), SensitiveAction::RestoreVault)
            .unwrap_err(),
        SessionError::ClockRollback
    );
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
}
