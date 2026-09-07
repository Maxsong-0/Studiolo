//! 请求必须各自满足可信 Access、严格浏览器来源及唯一安全头，不能复用先前身份。

use super::{
    support::*,
    test_support::{claims, signed},
};
use axum::http::{HeaderName, HeaderValue};
use serde_json::json;

#[tokio::test]
async fn every_request_rechecks_signed_identity_and_owner_even_with_valid_cookie() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    for (field, value) in [
        ("sub", json!("other-person")),
        ("exp", json!(1)),
        ("iss", json!("https://other.cloudflareaccess.com")),
        ("aud", json!(["other"])),
        ("email", json!("other@example.com")),
        ("type", json!("service")),
    ] {
        let mut payload = claims();
        payload[field] = value;
        let jwt = signed(&payload);
        let mut request = f.request("GET", OWN, Some(&issued.cookie), "");
        request
            .headers_mut()
            .insert("cf-access-jwt-assertion", jwt.parse().unwrap());
        let response = f.send(request, 401).await;
        if field != "sub" {
            // 验证器拒绝不触碰可能仍有效的应用Cookie；有效签名的错误主人另由Auth拒绝。
            assert!(!response.headers().contains_key("set-cookie"));
        }
        let body = json(response).await.to_string();
        for secret in [&jwt, &issued.token, &issued.csrf] {
            assert!(!body.contains(secret));
        }
    }
    let mut absent = f.request("GET", OWN, Some(&issued.cookie), "");
    absent.headers_mut().remove("cf-access-jwt-assertion");
    absent.headers_mut().insert(
        "cf-access-authenticated-user-email",
        "maxsong@example.com".parse().unwrap(),
    );
    let response = f.send(absent, 401).await;
    assert!(!response.headers().contains_key("set-cookie"));
    f.send(f.request("GET", OWN, Some(&issued.cookie), ""), 200)
        .await;
}

#[tokio::test]
async fn exact_browser_headers_apply_to_auth_and_existing_content_routes() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    for path in [OWN, "/api/workbench"] {
        for (name, bad) in [
            ("x-studiolo-request", "0"),
            ("sec-fetch-site", "same-site"),
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-site", "none"),
            ("sec-fetch-mode", "navigate"),
            ("sec-fetch-dest", "document"),
            ("sec-fetch-user", "?1"),
            ("origin", "null"),
            ("origin", "https://wrong.example.com"),
            ("host", "localhost:4319"),
        ] {
            let mut request = f.request("GET", path, Some(&issued.cookie), "");
            request.headers_mut().insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                bad.parse().unwrap(),
            );
            f.send(request, 403).await;
        }
        for name in [
            "x-studiolo-request",
            "sec-fetch-site",
            "sec-fetch-mode",
            "sec-fetch-dest",
        ] {
            let mut request = f.request("GET", path, Some(&issued.cookie), "");
            request.headers_mut().remove(name);
            f.send(request, 403).await;
        }
        for mode in ["cors", "same-origin"] {
            let mut request = f.request("GET", path, Some(&issued.cookie), "");
            request
                .headers_mut()
                .insert("sec-fetch-mode", mode.parse().unwrap());
            request
                .headers_mut()
                .insert("origin", ORIGIN.parse().unwrap());
            f.send(request, 200).await;
        }
    }
}

#[tokio::test]
async fn issuance_requires_origin_even_without_a_previous_cookie() {
    let f = Fixture::new(true).await;
    for origin in [None, Some("null"), Some("https://wrong.example.com")] {
        let mut request = f.request("POST", SESSIONS, None, "{}");
        request.headers_mut().remove("origin");
        if let Some(value) = origin {
            request
                .headers_mut()
                .insert("origin", value.parse().unwrap());
        }
        f.send(request, 403).await;
    }
    assert!(f.events("session.issue").is_empty());
    f.issue().await;
}

#[tokio::test]
async fn duplicate_security_headers_are_rejected_before_any_issuance() {
    let f = Fixture::new(true).await;
    for name in [
        "host",
        "origin",
        "cf-access-jwt-assertion",
        "x-studiolo-request",
        "sec-fetch-site",
        "sec-fetch-mode",
        "sec-fetch-dest",
        "sec-fetch-user",
        "cookie",
        "x-csrf-token",
    ] {
        let mut request = f.request("POST", SESSIONS, None, "{}");
        let key = HeaderName::from_bytes(name.as_bytes()).unwrap();
        let value = request
            .headers()
            .get(&key)
            .cloned()
            .unwrap_or(HeaderValue::from_static("fixture"));
        if !request.headers().contains_key(&key) {
            request.headers_mut().insert(key.clone(), value.clone());
        }
        request.headers_mut().append(key, value);
        f.send(request, 403).await;
    }
    assert!(f.events("session.issue").is_empty());
}

#[tokio::test]
async fn cookie_pair_binding_is_required_and_ambiguity_never_selects_a_favorite() {
    let f = Fixture::new(true).await;
    let first = f.issue().await;
    let second = f.issue().await;
    let swapped = format!("{COOKIE}={}~{}", first.token, second.csrf);
    let response = f.send(f.request("GET", OWN, Some(&swapped), ""), 401).await;
    cleared(&response);
    for (cookie, status) in [
        (format!("{}; {}", first.cookie, second.cookie), 400),
        (format!("{}; {}", first.cookie, first.cookie), 400),
        (format!("{COOKIE}=invalid"), 401),
        (format!("{COOKIE}=\"{}~{}\"", first.token, first.csrf), 400),
        (
            format!("{COOKIE}={}~{}~extra", first.token, first.csrf),
            401,
        ),
    ] {
        f.send(f.request("GET", OWN, Some(&cookie), ""), status)
            .await;
    }
    let with_access = format!("CF_Authorization=fictional; {}; other=valid", first.cookie);
    f.send(f.request("GET", OWN, Some(&with_access), ""), 200)
        .await;
    f.send(f.request("GET", OWN, Some(&first.cookie), ""), 200)
        .await;
}
