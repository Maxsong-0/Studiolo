//! 128 次真实提权检验同一长寿会话的有界替换；不是 T-014 的 1026 次签发测试。

use super::*;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn repeated_step_up_on_one_long_lived_session_keeps_only_current_proof() {
    let h = TotpHarness::enrolled(3600, 604_800);
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    for index in 0..128 {
        // 每次跨过完整限流窗口；idle 仍连续有效，absolute 不随操作延长。
        let now = NEXT + index * 900_000;
        h.h.clock.set(now);
        let proof = h
            .totp
            .step_up(&request(&owner, &issued, 1), code(now))
            .unwrap();
        assert_eq!(proof.verified_at_ms, now);
        assert_eq!(proof.expires_at_ms, now + 300_000);
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_control"), 1);
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_factor"), 1);
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 1);
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_pending"), 0);
        assert!(scalar(&h.h.manager, "SELECT count(*) FROM totp_rates") <= 66);
        assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM sessions"), 1);
    }
    let (manager, totp, _) = h.reopen();
    totp.require_l3(
        &request(&owner, &issued, 1),
        SensitiveAction::ActivatePolicy,
    )
    .unwrap();
    let records =
        h.h.audit
            .query(&AuditFilter::default().with_action("totp.step_up"))
            .unwrap();
    assert_eq!(records.len(), 128 * 2);
    for record in records {
        record.verify_integrity().unwrap();
    }
    manager
        .revoke_all(&owner, &issued.token, &issued.csrf)
        .unwrap();
    assert_eq!(manager.maintain_retention().unwrap().reclaimed_sessions, 1);
    assert_eq!(scalar(&manager, "SELECT count(*) FROM totp_grants"), 0);
    assert_eq!(scalar(&manager, "SELECT count(*) FROM totp_factor"), 1);
    assert_eq!(
        scalar(&manager, "SELECT last_step FROM totp_factor"),
        (NEXT + 127 * 900_000) / 30_000
    );
}
