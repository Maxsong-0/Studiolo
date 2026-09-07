//! 真实配对与显式 CSRF 下的当前、指定和全部撤权；撤权结果不暴露凭证。

use super::support::*;
use serde_json::json;

#[tokio::test]
async fn automatically_sent_cookie_csrf_never_substitutes_for_the_explicit_header() {
    let f = Fixture::new(true).await;
    let first = f.issue().await;
    let other = f.issue().await;
    for csrf in [
        None,
        Some("invalid"),
        Some(first.token.as_str()),
        Some(other.csrf.as_str()),
    ] {
        let mut request = f.request(
            "POST",
            REVOKE,
            Some(&first.cookie),
            "{\"scope\":\"current\"}",
        );
        if let Some(csrf) = csrf {
            request
                .headers_mut()
                .insert("x-csrf-token", csrf.parse().unwrap());
        }
        let response = f.send(request, 403).await;
        assert!(!response.headers().contains_key("set-cookie"));
    }
    let mut duplicate = f.request(
        "POST",
        REVOKE,
        Some(&first.cookie),
        "{\"scope\":\"current\"}",
    );
    duplicate
        .headers_mut()
        .append("x-csrf-token", first.csrf.parse().unwrap());
    duplicate
        .headers_mut()
        .append("x-csrf-token", first.csrf.parse().unwrap());
    f.send(duplicate, 403).await;
    assert!(f.events("session.revoked").is_empty());
    f.send(f.request("GET", OWN, Some(&first.cookie), ""), 200)
        .await;
}

#[tokio::test]
async fn current_logout_clears_cookie_and_the_old_credential_cannot_retry_as_authorized() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
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
    assert_eq!(json(response).await, json!({"revoked":1}));
    let response = f
        .send(f.request("GET", OWN, Some(&issued.cookie), ""), 401)
        .await;
    cleared(&response);
    let mut retry = f.request(
        "POST",
        REVOKE,
        Some(&issued.cookie),
        "{\"scope\":\"current\"}",
    );
    retry
        .headers_mut()
        .insert("x-csrf-token", issued.csrf.parse().unwrap());
    f.send(retry, 401).await;
    assert_eq!(f.events("session.revoked").len(), 2);
    f.send(f.request("POST", SESSIONS, Some(&issued.cookie), "{}"), 409)
        .await;
}

#[tokio::test]
async fn specific_target_only_revokes_that_session_and_self_target_clears_cookie() {
    let f = Fixture::new(true).await;
    let first = f.issue().await;
    let target = f.issue().await;
    let body = json!({"scope":"session","session_id":target.id}).to_string();
    for expected in [1, 0] {
        let mut request = f.request("POST", REVOKE, Some(&first.cookie), &body);
        request
            .headers_mut()
            .insert("x-csrf-token", first.csrf.parse().unwrap());
        let response = f.send(request, 200).await;
        assert!(!response.headers().contains_key("set-cookie"));
        assert_eq!(json(response).await, json!({"revoked":expected}));
    }
    f.send(f.request("GET", OWN, Some(&target.cookie), ""), 401)
        .await;
    f.send(f.request("GET", OWN, Some(&first.cookie), ""), 200)
        .await;
    let body = json!({"scope":"session","session_id":first.id}).to_string();
    let mut request = f.request("POST", REVOKE, Some(&first.cookie), &body);
    request
        .headers_mut()
        .insert("x-csrf-token", first.csrf.parse().unwrap());
    let response = f.send(request, 200).await;
    cleared(&response);
    assert_eq!(json(response).await, json!({"revoked":1}));
    assert_eq!(f.events("session.revoked").len(), 4);
}

#[tokio::test]
async fn all_revocation_invalidates_every_session_and_requires_explicit_cookie_free_login() {
    let f = Fixture::new(true).await;
    let first = f.issue().await;
    let second = f.issue().await;
    let mut request = f.request("POST", REVOKE, Some(&first.cookie), "{\"scope\":\"all\"}");
    request
        .headers_mut()
        .insert("x-csrf-token", first.csrf.parse().unwrap());
    let response = f.send(request, 200).await;
    cleared(&response);
    assert_eq!(json(response).await, json!({"revoked":2}));
    for cookie in [&first.cookie, &second.cookie] {
        f.send(f.request("GET", OWN, Some(cookie), ""), 401).await;
        f.send(f.request("POST", SESSIONS, Some(cookie), "{}"), 409)
            .await;
    }
    let next = f.issue().await;
    assert_ne!(next.id, first.id);
    assert_ne!(next.id, second.id);
}
