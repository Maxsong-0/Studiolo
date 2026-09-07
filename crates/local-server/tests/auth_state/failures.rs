use super::{common::*, fixture::Fixture, support::*};
use std::fs;

#[tokio::test]
async fn broken_storage_returns_unavailable_instead_of_unconfigured_or_logged_out() {
    let mut fixture = Fixture::new(true).await;
    let login = fixture.issue().await;
    let database = fixture.root.join("auth/auth.sqlite3");
    let original = fixture.root.join("auth/original.sqlite3");
    fs::rename(&database, &original).unwrap();
    fs::write(&database, "state-bad-database-private-canary").unwrap();
    mode(&database, 0o600);
    let reply = fixture.send(request(&fixture, Some(&login)), 503).await;
    reply.kept_cookie();
    keys(&reply.json(), &["error"]);
    no_secrets(&reply, &login, &["state-bad-database-private-canary"]);
    fs::remove_file(&database).unwrap();
    fs::rename(&original, &database).unwrap();
    state(&fixture, &login).await;
    fixture.stop().await;
}

#[tokio::test]
async fn failed_factor_preserves_session_but_revoked_state_clears_cookie() {
    let mut fixture = Fixture::new(true).await;
    let login = fixture.issue().await;
    let body = format!("{{\"code\":\"{}\"}}", wrong(&OLD, fixture.now()));
    fixture
        .send(fixture.request("POST", STEP_UP, Some(&login), &body), 403)
        .await
        .kept_cookie();
    assert!(state(&fixture, &login).await["recent_authentication"].is_null());
    fixture
        .send(
            fixture.request("POST", REVOKE, Some(&login), "{\"scope\":\"current\"}"),
            200,
        )
        .await
        .cleared_cookie();
    let reply = fixture.send(request(&fixture, Some(&login)), 401).await;
    reply.cleared_cookie();
    no_secrets(&reply, &login, &[]);
    keys(&reply.json(), &["error"]);
    fixture.stop().await;
}
