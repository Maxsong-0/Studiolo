//! 恢复预算独立于 TOTP；新 Cookie、来源变化及重启均不能刷新账号窗口。

use super::*;

#[test]
fn account_budget_survives_cookie_source_restart_and_opens_only_at_exact_boundary() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (_, codes) = h.ready();
    let prepared = h.prepared();
    h.h.clock.set(CHANGE);
    for source in 1..=5 {
        let issued = h.h.manager.issue(&owner).unwrap();
        assert_eq!(
            h.recovery
                .replace_factor(
                    &request(&owner, &issued, source),
                    invalid_code(&codes),
                    &prepared,
                    otp(&NEW, CHANGE)
                )
                .err()
                .unwrap(),
            SessionError::Rejected
        );
    }
    let (manager, recovery) = h.reopen();
    let issued = manager.issue(&owner).unwrap();
    let prepared = PreparedTotpFactor::prepare(manager.clone(), seed(NEW)).unwrap();
    for now in [CHANGE, CHANGE + 899_999] {
        h.h.clock.set(now);
        assert_eq!(
            recovery
                .replace_factor(
                    &request(&owner, &issued, 99),
                    copy_code(&codes, 0),
                    &prepared,
                    otp(&NEW, now)
                )
                .err()
                .unwrap(),
            SessionError::RateLimited
        );
    }
    assert_eq!(
        scalar(&manager, "SELECT used_mask FROM recovery_control"),
        0
    );
    assert_eq!(scalar(&manager, "SELECT count(*) FROM recovery_pending"), 0);
    h.h.clock.set(CHANGE + 900_000);
    recovery
        .replace_factor(
            &request(&owner, &issued, 99),
            copy_code(&codes, 0),
            &prepared,
            otp(&NEW, CHANGE + 900_000),
        )
        .unwrap();
}

#[test]
fn source_limit_is_independent_and_ipv4_mapped_address_is_not_an_extra_bucket() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (issued, codes) = h.ready();
    let prepared = h.prepared();
    h.h.clock.set(CHANGE);
    assert!(
        h.recovery
            .replace_factor(
                &request(&owner, &issued, 2),
                invalid_code(&codes),
                &prepared,
                otp(&NEW, CHANGE)
            )
            .is_err()
    );
    h.h.clock.set(CHANGE + 60_000);
    for _ in 0..4 {
        assert_eq!(
            h.recovery
                .replace_factor(
                    &request(&owner, &issued, 1),
                    invalid_code(&codes),
                    &prepared,
                    otp(&NEW, CHANGE + 60_000)
                )
                .err()
                .unwrap(),
            SessionError::Rejected
        );
    }
    let now = CHANGE + 900_000;
    h.h.clock.set(now);
    let (_, recovery) = h.reopen();
    assert_eq!(
        recovery
            .replace_factor(
                &request(&owner, &issued, 1),
                invalid_code(&codes),
                &prepared,
                otp(&NEW, now)
            )
            .err()
            .unwrap(),
        SessionError::Rejected
    );
    let mapped = TotpRequest {
        source: "::ffff:192.0.2.1".parse().unwrap(),
        ..request(&owner, &issued, 1)
    };
    assert_eq!(
        recovery
            .replace_factor(&mapped, copy_code(&codes, 0), &prepared, otp(&NEW, now))
            .err()
            .unwrap(),
        SessionError::RateLimited
    );
    recovery
        .replace_factor(
            &request(&owner, &issued, 3),
            copy_code(&codes, 0),
            &prepared,
            otp(&NEW, now),
        )
        .unwrap();
}

#[test]
fn wrong_new_otp_does_not_burn_a_valid_recovery_code_and_success_keeps_budget() {
    let h = RecoveryHarness::new(3600, 7200);
    let owner = identity();
    let (issued, codes) = h.ready();
    let prepared = h.prepared();
    h.h.clock.set(CHANGE);
    for _ in 0..4 {
        assert_eq!(
            h.recovery
                .replace_factor(
                    &request(&owner, &issued, 1),
                    copy_code(&codes, 0),
                    &prepared,
                    wrong_otp(CHANGE)
                )
                .err()
                .unwrap(),
            SessionError::Rejected
        );
    }
    assert_eq!(
        scalar(&h.h.manager, "SELECT used_mask FROM recovery_control"),
        0
    );
    assert_eq!(
        scalar(&h.h.manager, "SELECT high_step FROM recovery_control"),
        -1
    );
    h.recovery
        .replace_factor(
            &request(&owner, &issued, 1),
            copy_code(&codes, 0),
            &prepared,
            otp(&NEW, CHANGE),
        )
        .unwrap();
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT attempts FROM recovery_rates WHERE bucket='account'"
        ),
        5
    );
    let (manager, _) = h.reopen();
    assert_eq!(
        scalar(
            &manager,
            "SELECT attempts FROM recovery_rates WHERE bucket='account'"
        ),
        5
    );
}

#[test]
fn parse_rejects_noncanonical_codes_without_touching_budget() {
    let h = RecoveryHarness::new(3600, 7200);
    let (_, codes) = h.ready();
    let valid = codes.codes()[0].expose_secret();
    let malformed = [
        String::new(),
        format!(" {valid}"),
        valid.to_uppercase(),
        valid[..54].into(),
        format!("{}0a{}", &valid[..20], &valid[22..]),
        "恢复码".repeat(30),
    ];
    for text in malformed {
        assert!(RecoveryCode::parse(text).is_err());
    }
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT coalesce(sum(attempts),0) FROM recovery_rates"
        ),
        0
    );
}
