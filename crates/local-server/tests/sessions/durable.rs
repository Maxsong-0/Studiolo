//! 双 SQLite 重开和真实权限故障；失败不会把未登记或待定因子变成可签发状态。

use super::support::*;
use studiolo_auth::Totp;

#[tokio::test]
async fn absent_and_only_pending_factor_cannot_become_a_remote_registration() {
    let f = Fixture::new(false).await;
    f.send(f.request("POST", SESSIONS, None, "{}"), 409).await;
    assert!(f.events("session.issue").is_empty());
    f.pending_enrollment();
    f.send(f.request("POST", SESSIONS, None, "{}"), 409).await;
    assert!(f.events("session.issue").is_empty());
    let (totp, _) = Totp::attach(f.manager(), seed()).unwrap();
    totp.flush_audit().unwrap();
    f.send(f.request("POST", SESSIONS, None, "{}"), 409).await;
    f.advance(300_000);
    totp.flush_audit().unwrap();
    f.send(f.request("POST", SESSIONS, None, "{}"), 409).await;
    assert!(f.events("session.issue").is_empty());
    assert_eq!(f.events("totp.enrolled").len(), 2);
}

#[tokio::test]
async fn disk_reopen_keeps_the_pair_but_idle_expiry_never_automatically_renews() {
    let mut f = Fixture::new(true).await;
    let issued = f.issue().await;
    let original_events = f.events("session.issue");
    f.reopen().await;
    let response = f
        .send(f.request("GET", OWN, Some(&issued.cookie), ""), 200)
        .await;
    assert_eq!(json(response).await["csrf_token"], issued.csrf);
    assert_eq!(f.events("session.issue"), original_events);
    f.advance(600_000);
    f.reopen().await;
    let response = f
        .send(f.request("GET", OWN, Some(&issued.cookie), ""), 401)
        .await;
    cleared(&response);
    f.send(f.request("POST", SESSIONS, Some(&issued.cookie), "{}"), 409)
        .await;
    assert_eq!(f.events("session.issue"), original_events);
    let next = f.issue().await;
    assert_ne!(next.token, issued.token);
    assert_ne!(next.csrf, issued.csrf);
    assert_ne!(next.id, issued.id);
}

#[tokio::test]
async fn activity_cannot_extend_the_original_absolute_deadline() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    let first = json(
        f.send(f.request("GET", OWN, Some(&issued.cookie), ""), 200)
            .await,
    )
    .await;
    let expiry = first["session"]["absolute_expires_at_ms"].as_i64().unwrap();
    for _ in 0..3 {
        f.advance(500_000);
        let body = json(
            f.send(f.request("GET", OWN, Some(&issued.cookie), ""), 200)
                .await,
        )
        .await;
        assert_eq!(body["session"]["absolute_expires_at_ms"], expiry);
    }
    f.advance(300_000);
    assert_eq!(f.now(), expiry);
    f.send(f.request("GET", OWN, Some(&issued.cookie), ""), 401)
        .await;
    assert_eq!(f.events("session.issue").len(), 2);
}

#[tokio::test]
async fn audit_outage_blocks_new_authority_but_not_reading_or_persistent_logout() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    let audit = f.audit.join("audit.sqlite3");
    mode(&audit, 0o400);
    let failed = f.send(f.request("POST", SESSIONS, None, "{}"), 503).await;
    assert!(!failed.headers().contains_key("set-cookie"));
    f.send(f.request("GET", OWN, Some(&issued.cookie), ""), 200)
        .await;
    f.send(
        f.request("GET", "/api/workbench", Some(&issued.cookie), ""),
        200,
    )
    .await;
    let mut request = f.request(
        "POST",
        REVOKE,
        Some(&issued.cookie),
        "{\"scope\":\"current\"}",
    );
    request
        .headers_mut()
        .insert("x-csrf-token", issued.csrf.parse().unwrap());
    let response = f.send(request, 200).await;
    cleared(&response);
    assert_eq!(json(response).await["revoked"], 1);
    f.send(f.request("GET", OWN, Some(&issued.cookie), ""), 401)
        .await;
    mode(&audit, 0o600);
    assert!(f.events("session.revoked").is_empty());
    f.manager().flush_audit(256).unwrap();
    let events = f.events("session.revoked");
    assert_eq!(events.len(), 2);
    f.manager().flush_audit(256).unwrap();
    assert_eq!(f.events("session.revoked"), events);
    assert_eq!(f.events("session.issue").len(), 2);
}

#[tokio::test]
async fn auth_storage_fault_is_not_disguised_as_logout_or_a_cookie_clear() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    let auth = f.auth.join("auth.sqlite3");
    mode(&auth, 0o400);
    for path in [OWN, "/api/workbench"] {
        let response = f
            .send(f.request("GET", path, Some(&issued.cookie), ""), 503)
            .await;
        assert!(!response.headers().contains_key("set-cookie"));
    }
    let mut request = f.request("POST", REVOKE, Some(&issued.cookie), "{\"scope\":\"all\"}");
    request
        .headers_mut()
        .insert("x-csrf-token", issued.csrf.parse().unwrap());
    let response = f.send(request, 503).await;
    assert!(!response.headers().contains_key("set-cookie"));
    mode(&auth, 0o600);
    assert!(f.events("session.revoked").is_empty());
    f.send(f.request("GET", OWN, Some(&issued.cookie), ""), 200)
        .await;
}
