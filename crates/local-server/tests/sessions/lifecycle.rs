//! 新组合入口完整链及旧 M1 隔离；不把 Cookie 元信息当作可缓存授权。

use super::{
    support::*,
    test_support::{claims, signed, verifier},
};
use axum::{body::Body, http::Request};
use studiolo_vault_broker::ReadOnlyVault;
use tower::ServiceExt;

#[tokio::test]
async fn issued_cookie_reads_own_csrf_list_and_every_content_surface() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    let own = f
        .send(f.request("GET", OWN, Some(&issued.cookie), ""), 200)
        .await;
    let own = json(own).await;
    assert_eq!(own["session"]["id"], issued.id);
    assert_eq!(own["csrf_token"], issued.csrf);
    assert!(!own.to_string().contains(&issued.token));
    let list = json(
        f.send(f.request("GET", SESSIONS, Some(&issued.cookie), ""), 200)
            .await,
    )
    .await;
    assert_eq!(list["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(list["sessions"][0]["id"], issued.id);
    assert!(!list.to_string().contains(&issued.token));
    assert!(!list.to_string().contains(&issued.csrf));
    for path in [
        "/api/workbench",
        "/api/workbench?refresh=true",
        "/api/document?path=a.md&generation=2",
        "/api/tree?path=&offset=0&generation=2",
        "/api/search?q=reading&mode=full_text&generation=2",
    ] {
        f.send(f.request("GET", path, Some(&issued.cookie), ""), 200)
            .await;
    }
    assert_eq!(f.events("session.issue").len(), 2);
    assert!(f.events("totp.step_up").is_empty());
}

#[tokio::test]
async fn api_requires_application_cookie_while_static_bootstrap_requires_only_access() {
    let f = Fixture::new(true).await;
    for path in [
        OWN,
        SESSIONS,
        "/api/workbench",
        "/api/workbench?refresh=true",
        "/api/document?path=a.md&generation=1",
        "/api/tree?path=&offset=0&generation=1",
        "/api/search?q=reading&mode=full_text&generation=1",
    ] {
        f.send(f.request("GET", path, None, ""), 401).await;
    }
    for path in ["/", "/assets/app.js", "/assets/app.css"] {
        let request = Request::builder()
            .uri(path)
            .header("host", "127.0.0.1:4319")
            .header("cf-access-jwt-assertion", signed(&claims()))
            .body(Body::empty())
            .unwrap();
        f.send(request, 200).await;
        let request = Request::builder()
            .uri(path)
            .header("host", "127.0.0.1:4319")
            .body(Body::empty())
            .unwrap();
        f.send(request, 401).await;
    }
}

#[tokio::test]
async fn existing_cookie_never_silently_issues_another_session() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    for cookie in [&issued.cookie, &format!("{COOKIE}=invalid")] {
        let response = f
            .send(f.request("POST", SESSIONS, Some(cookie), "{}"), 409)
            .await;
        assert!(!response.headers().contains_key("set-cookie"));
    }
    assert_eq!(f.events("session.issue").len(), 2);
    f.send(f.request("GET", OWN, Some(&issued.cookie), ""), 200)
        .await;
}

#[tokio::test]
async fn ordinary_and_access_only_routers_keep_their_original_read_only_contract() {
    let f = Fixture::new(true).await;
    for public in [false, true] {
        let vault = ReadOnlyVault::open(&f.root.join("vault")).unwrap();
        let assets = ReadOnlyVault::open(&f.root.join("web")).unwrap();
        let app = if public {
            studiolo_local_server::router_with_access(
                vault,
                assets,
                "虚构".into(),
                4319,
                verifier().await,
            )
            .unwrap()
        } else {
            studiolo_local_server::router(vault, assets, "虚构".into(), 4319).unwrap()
        };
        for (method, path, expected) in [
            ("GET", "/api/workbench", 200),
            ("HEAD", "/api/workbench", 200),
            ("POST", SESSIONS, 405),
            ("POST", REVOKE, 405),
        ] {
            let mut request = Request::builder()
                .method(method)
                .uri(path)
                .header("host", "127.0.0.1:4319");
            if public {
                request = request.header("cf-access-jwt-assertion", signed(&claims()));
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), expected);
            secure(&response);
        }
    }
    assert!(f.events("session.issue").is_empty());
}

#[tokio::test]
async fn auth_routes_have_no_query_or_head_and_no_write_capability_is_added_to_content() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    for path in [OWN, SESSIONS, REVOKE] {
        let response = f
            .send(f.request("HEAD", path, Some(&issued.cookie), ""), 405)
            .await;
        let allow = if path == OWN {
            "GET"
        } else if path == SESSIONS {
            "GET, POST"
        } else {
            "POST"
        };
        assert_eq!(response.headers()["allow"], allow);
        assert!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .is_empty()
        );
    }
    for path in [
        format!("{OWN}?x=1"),
        format!("{SESSIONS}?"),
        format!("{SESSIONS}?x=1"),
    ] {
        f.send(f.request("GET", &path, Some(&issued.cookie), ""), 400)
            .await;
    }
    f.send(
        f.request("POST", "/api/workbench", Some(&issued.cookie), "{}"),
        405,
    )
    .await;
    assert_eq!(
        std::fs::read_to_string(f.root.join("vault/a.md")).unwrap(),
        "# 示例\nreading\n"
    );
}
