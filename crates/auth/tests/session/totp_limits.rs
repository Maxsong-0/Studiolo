//! 限流跨 Cookie、可信来源和重启；正确验证码不退款，也不清空先前失败。

use super::*;

#[test]
fn account_budget_cannot_be_reset_by_new_cookie_source_or_restart() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let now = START + 900_000;
    h.h.clock.set(now);
    for source in 1..=5 {
        let issued = h.h.manager.issue(&owner).unwrap();
        assert_eq!(
            h.totp
                .step_up(&request(&owner, &issued, source), wrong_code(now))
                .unwrap_err(),
            SessionError::Rejected
        );
    }
    let (manager, totp, _) = h.reopen();
    let fresh = manager.issue(&owner).unwrap();
    assert_eq!(
        totp.step_up(&request(&owner, &fresh, 99), code(now))
            .unwrap_err(),
        SessionError::RateLimited
    );
    h.h.clock.set(now + 899_999);
    assert_eq!(
        totp.step_up(&request(&owner, &fresh, 100), code(now + 899_999))
            .unwrap_err(),
        SessionError::RateLimited
    );
    h.h.clock.set(now + 900_000);
    totp.step_up(&request(&owner, &fresh, 100), code(now + 900_000))
        .unwrap();
}

#[test]
fn source_window_remains_exhausted_after_account_window_resets() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    // 登记先建立账号窗口；网络来源窗口晚一分钟建立，故能独立观察来源限制。
    for _ in 0..4 {
        assert_eq!(
            h.totp
                .step_up(&request(&owner, &issued, 1), wrong_code(NEXT))
                .unwrap_err(),
            SessionError::Rejected
        );
    }
    let now = START + 900_000;
    h.h.clock.set(now);
    let (_, totp, _) = h.reopen();
    assert_eq!(
        totp.step_up(&request(&owner, &issued, 1), wrong_code(now))
            .unwrap_err(),
        SessionError::Rejected
    );
    let mapped = TotpRequest {
        source: "::ffff:192.0.2.1".parse().unwrap(),
        ..request(&owner, &issued, 1)
    };
    assert_eq!(
        totp.step_up(&mapped, code(now)).unwrap_err(),
        SessionError::RateLimited
    );
    totp.step_up(&request(&owner, &issued, 2), code(now))
        .unwrap();
}

#[test]
fn successful_verification_does_not_refund_or_reset_five_admitted_attempts() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let now = START + 900_000;
    h.h.clock.set(now);
    let issued = h.h.manager.issue(&owner).unwrap();
    for _ in 0..2 {
        assert_eq!(
            h.totp
                .step_up(&request(&owner, &issued, 1), wrong_code(now))
                .unwrap_err(),
            SessionError::Rejected
        );
    }
    h.totp
        .step_up(&request(&owner, &issued, 1), code(now))
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            h.totp
                .step_up(&request(&owner, &issued, 1), wrong_code(now))
                .unwrap_err(),
            SessionError::Rejected
        );
    }
    h.h.clock.set(now + 30_000);
    assert_eq!(
        h.totp
            .step_up(&request(&owner, &issued, 2), code(now + 30_000))
            .unwrap_err(),
        SessionError::RateLimited
    );
    h.totp
        .require_l3(&request(&owner, &issued, 1), SensitiveAction::ChangeBackup)
        .unwrap();
}

#[test]
fn malformed_codes_are_rejected_before_any_budget_or_factor_storage_change() {
    let h = TotpHarness::enrolled(3600, 7200);
    let before = scalar(
        &h.h.manager,
        "SELECT coalesce(sum(attempts),0) FROM totp_rates",
    );
    for invalid in [
        "",
        "12345",
        "1234567",
        " 123456",
        "１２３４５６",
        "12345a",
        "123456\n",
    ] {
        assert!(TotpCode::parse(invalid.to_owned()).is_err());
    }
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT coalesce(sum(attempts),0) FROM totp_rates"
        ),
        before
    );
}
