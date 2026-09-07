//! 真实恢复完成后的当前因子锚点可通过启动预检，旧登记记录无需成为当前证明。

use super::*;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

#[test]
fn readiness_accepts_current_recovered_factor_without_consumption_or_new_session() {
    let h = RecoveryHarness::new(3600, 7200);
    let (issued, codes) = h.ready();
    h.h.clock.set(CHANGE);
    h.recovery
        .replace_factor(
            &request(&identity(), &issued, 1),
            copy_code(&codes, 0),
            &h.prepared(),
            otp(&NEW, CHANGE),
        )
        .unwrap();
    let before = h.h.audit.query(&AuditFilter::default()).unwrap();
    let consumed = scalar(&h.h.manager, "SELECT last_step FROM totp_factor");
    let next = scalar(&h.h.manager, "SELECT next_issuance FROM credential_state");
    let (reopened, _) = h.reopen();
    reopened.require_http_ready().unwrap();
    assert_eq!(
        scalar(&reopened, "SELECT last_step FROM totp_factor"),
        consumed
    );
    assert_eq!(
        scalar(&reopened, "SELECT next_issuance FROM credential_state"),
        next
    );
    assert_eq!(
        scalar(
            &reopened,
            "SELECT count(*) FROM sessions WHERE state='active'"
        ),
        0
    );
    assert_eq!(h.h.audit.query(&AuditFilter::default()).unwrap(), before);
}
