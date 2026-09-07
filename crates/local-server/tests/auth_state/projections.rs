use super::{common::*, fixture::Fixture, support::*};

#[tokio::test]
async fn state_tracks_three_factor_actions_without_returning_their_secrets() {
    let mut fixture = Fixture::new(true).await;
    let first = fixture.issue().await;
    let second = fixture.issue().await;
    let initial = state(&fixture, &first).await;
    assert!(initial["recent_authentication"].is_null());
    assert_eq!(initial["recovery_candidate_available"], true);
    assert_eq!(initial["pending"], false);
    // 普通状态读取不能消费随后使用的 OTP。
    state(&fixture, &first).await;
    fixture.elevate(&first, false).await;
    let elevated = state(&fixture, &first).await;
    assert_eq!(
        elevated["recent_authentication"]["verified_at_ms"],
        fixture.now()
    );
    assert_eq!(
        elevated["recent_authentication"]["expires_at_ms"],
        fixture.now() + 300_000
    );
    assert!(state(&fixture, &second).await["recent_authentication"].is_null());

    let codes = fixture
        .send(fixture.request("POST", CODES, Some(&first), "{}"), 200)
        .await
        .json();
    let codes = codes["codes"].as_array().unwrap();
    let after_codes = state(&fixture, &first).await;
    assert_eq!(
        after_codes["recent_authentication"],
        elevated["recent_authentication"]
    );
    assert_eq!(after_codes["pending"], false);
    let reply = fixture.send(request(&fixture, Some(&first)), 200).await;
    let secrets = codes
        .iter()
        .map(|code| code.as_str().unwrap())
        .collect::<Vec<_>>();
    no_secrets(&reply, &first, &secrets);

    fixture.advance(60_000);
    let body =
        serde_json::json!({"recovery_code":codes[0], "code":otp(&NEW, fixture.now())}).to_string();
    fixture
        .send(fixture.request("POST", RECOVER, Some(&first), &body), 204)
        .await
        .cleared_cookie();
    for old in [&first, &second] {
        fixture
            .send(request(&fixture, Some(old)), 401)
            .await
            .cleared_cookie();
    }
    let fresh = fixture.issue().await;
    let recovered = state(&fixture, &fresh).await;
    assert!(recovered["recent_authentication"].is_null());
    assert_eq!(recovered["recovery_candidate_available"], false);
    assert_eq!(recovered["pending"], false);
    assert_eq!(fixture.events("totp.step_up").len(), 2);
    assert_eq!(fixture.events("recovery.codes_rotated").len(), 2);
    assert_eq!(fixture.events("totp.recovered").len(), 2);
    fixture.stop().await;
}

#[tokio::test]
async fn missing_candidate_and_expired_recent_authentication_are_explicit_states() {
    let mut fixture = Fixture::new(false).await;
    let login = fixture.issue().await;
    assert_eq!(
        state(&fixture, &login).await["recovery_candidate_available"],
        false
    );
    fixture.elevate(&login, false).await;
    fixture.advance(300_000);
    let current = state(&fixture, &login).await;
    assert!(current["recent_authentication"].is_null());
    assert_eq!(current["pending"], false);
    fixture.stop().await;
}

#[tokio::test]
async fn pending_projection_does_not_flush_or_retry_the_failed_factor_action() {
    let mut fixture = Fixture::new(true).await;
    let login = fixture.issue().await;
    let audit = fixture.root.join("audit/audit.sqlite3");
    mode(&audit, 0o400);
    let body = format!("{{\"code\":\"{}\"}}", otp(&OLD, fixture.now()));
    fixture
        .send(fixture.request("POST", STEP_UP, Some(&login), &body), 503)
        .await
        .kept_cookie();
    mode(&audit, 0o600);
    let events = fixture.events("totp.step_up");
    for _ in 0..2 {
        let current = state(&fixture, &login).await;
        assert_eq!(current["pending"], true);
        assert!(current["recent_authentication"].is_null());
    }
    assert_eq!(fixture.events("totp.step_up"), events);
    fixture.stop().await;
}
