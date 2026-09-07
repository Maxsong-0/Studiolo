//! 前端模式发现不签发身份、不读取内容，不能借此绕过会话门禁。

use super::{support::*, test_support::verifier};
use axum::{body::to_bytes, http::HeaderValue};
use serde_json::json as value;
use studiolo_vault_broker::ReadOnlyVault;
use tower::ServiceExt;

const BOOTSTRAP: &str = "/api/v1/bootstrap";

#[tokio::test]
async fn bootstrap_returns_only_explicit_mode_in_all_compositions() {
    let f = Fixture::new(true).await;
    let response = f.send(f.request("GET", BOOTSTRAP, None, ""), 200).await;
    assert!(!response.headers().contains_key("set-cookie"));
    assert_eq!(json(response).await, value!({"authentication":"session"}));
    for access in [false, true] {
        let vault = ReadOnlyVault::open(&f.root.join("vault")).unwrap();
        let assets = ReadOnlyVault::open(&f.root.join("web")).unwrap();
        let app = if access {
            studiolo_local_server::router_with_access(
                vault,
                assets,
                "测试".into(),
                4319,
                verifier().await,
            )
            .unwrap()
        } else {
            studiolo_local_server::router(vault, assets, "测试".into(), 4319).unwrap()
        };
        let response = app
            .clone()
            .oneshot(f.request("GET", BOOTSTRAP, None, ""))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        secure(&response);
        assert_eq!(
            json(response).await,
            value!({"authentication":if access {"access"} else {"local"}})
        );
        if access {
            let mut request = f.request("GET", BOOTSTRAP, None, "");
            request.headers_mut().remove("cf-access-jwt-assertion");
            assert_eq!(app.clone().oneshot(request).await.unwrap().status(), 401);
        }
        for method in ["HEAD", "POST", "OPTIONS"] {
            let mut request = f.request(method, BOOTSTRAP, None, "");
            request.headers_mut().remove("origin");
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), 405);
            assert_eq!(response.headers()["allow"], "GET");
            if method == "HEAD" {
                assert!(
                    to_bytes(response.into_body(), 1024)
                        .await
                        .unwrap()
                        .is_empty()
                );
            }
        }
    }
    assert!(f.events("session.issue").is_empty());
}

#[tokio::test]
async fn bootstrap_exempts_cookie_only_and_never_unlocks_content() {
    let f = Fixture::new(false).await;
    for cookie in [None, Some("__Host-studiolo-session=invalid")] {
        let response = f.send(f.request("GET", BOOTSTRAP, cookie, ""), 200).await;
        assert!(!response.headers().contains_key("set-cookie"));
        assert_eq!(json(response).await, value!({"authentication":"session"}));
        f.send(f.request("GET", "/api/workbench", cookie, ""), 401)
            .await;
    }
    let mut request = f.request("GET", BOOTSTRAP, None, "");
    request.headers_mut().remove("cf-access-jwt-assertion");
    f.send(request, 401).await;
    assert!(f.events("session.issue").is_empty());
}

#[tokio::test]
async fn bootstrap_enforces_fetch_metadata_and_header_budget_before_cookie_exemption() {
    let f = Fixture::new(true).await;
    for name in [
        "x-studiolo-request",
        "sec-fetch-site",
        "sec-fetch-mode",
        "sec-fetch-dest",
    ] {
        let mut request = f.request("GET", BOOTSTRAP, None, "");
        request.headers_mut().remove(name);
        f.send(request, 403).await;
    }
    for (name, value) in [
        ("origin", "https://other.example"),
        ("sec-fetch-site", "cross-site"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-dest", "document"),
    ] {
        let mut request = f.request("GET", BOOTSTRAP, None, "");
        request
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
        f.send(request, 403).await;
    }
    let mut duplicate = f.request("GET", BOOTSTRAP, None, "");
    duplicate
        .headers_mut()
        .append("x-studiolo-request", HeaderValue::from_static("1"));
    f.send(duplicate, 403).await;
    let mut oversized = f.request("GET", BOOTSTRAP, None, "");
    oversized.headers_mut().insert(
        "x-padding",
        HeaderValue::from_str(&"x".repeat(32768)).unwrap(),
    );
    f.send(oversized, 431).await;
}

#[tokio::test]
async fn bootstrap_rejects_query_and_methods_without_creating_sessions() {
    let f = Fixture::new(true).await;
    for path in [
        format!("{BOOTSTRAP}?"),
        format!("{BOOTSTRAP}?authentication=local"),
    ] {
        f.send(f.request("GET", &path, None, ""), 400).await;
    }
    for method in ["HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"] {
        let response = f.send(f.request(method, BOOTSTRAP, None, ""), 405).await;
        assert_eq!(response.headers()["allow"], "GET");
    }
    assert!(f.events("session.issue").is_empty());
}

#[tokio::test]
async fn bootstrap_can_report_mode_during_auth_storage_failure_but_content_stays_closed() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    mode(&f.auth.join("auth.sqlite3"), 0o400);
    let response = f
        .send(f.request("GET", BOOTSTRAP, Some(&issued.cookie), ""), 200)
        .await;
    assert_eq!(json(response).await, value!({"authentication":"session"}));
    f.send(f.request("GET", OWN, Some(&issued.cookie), ""), 503)
        .await;
    f.send(
        f.request("GET", "/api/workbench", Some(&issued.cookie), ""),
        503,
    )
    .await;
    mode(&f.auth.join("auth.sqlite3"), 0o600);
}
