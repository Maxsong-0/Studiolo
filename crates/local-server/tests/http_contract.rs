//! 本机首片只读 HTTP 契约，所有源数据均为临时虚构夹具。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::Value;
use std::fs;
use studiolo_local_server::router;
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;
use tower::ServiceExt;

const NOTE: &str = "---\nlifecycle: 活跃\n---\n# 测试项目\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | M0 |\n\n## 用户侧待办\n\n- 待验收：确认只读工作台\n";
const ICON: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 32 32\"/>";

fn fixture() -> (TempDir, TempDir, Router) {
    let vault = TempDir::new().unwrap();
    let assets = TempDir::new().unwrap();
    fs::create_dir_all(vault.path().join("项目/测试项目/00-总览")).unwrap();
    fs::write(vault.path().join("项目/测试项目/00-总览/INDEX.md"), NOTE).unwrap();
    fs::create_dir(assets.path().join("assets")).unwrap();
    fs::write(
        assets.path().join("index.html"),
        "<!doctype html><title>测试</title>",
    )
    .unwrap();
    fs::write(assets.path().join("assets/app.js"), "console.log('local')").unwrap();
    fs::write(assets.path().join("assets/favicon-fixture.svg"), ICON).unwrap();
    fs::create_dir(vault.path().join("assets")).unwrap();
    fs::write(vault.path().join("assets/private.svg"), "private-vault-svg").unwrap();
    let app = router(
        ReadOnlyVault::open(vault.path()).unwrap(),
        ReadOnlyVault::open(assets.path()).unwrap(),
        "测试副本".into(),
        4317,
    )
    .unwrap();
    (vault, assets, app)
}

fn request(uri: &str) -> axum::http::request::Builder {
    Request::builder().uri(uri).header("host", "127.0.0.1:4317")
}

async fn json(app: Router, uri: &str) -> Value {
    let response = app
        .oneshot(request(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(
        &to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn real_snapshot_and_source_are_same_generation_without_vault_mutation() {
    let (vault, _assets, app) = fixture();
    let before = fs::read(vault.path().join("项目/测试项目/00-总览/INDEX.md")).unwrap();
    let snapshot = json(app.clone(), "/api/workbench").await;
    assert_eq!(snapshot["generation"], 1);
    assert_eq!(snapshot["source_label"], "测试副本");
    assert_eq!(snapshot["projects"][0]["name"], "测试项目");
    let source = snapshot["projects"][0]["source"]["path"].as_str().unwrap();
    let encoded = source
        .bytes()
        .map(|b| format!("%{b:02X}"))
        .collect::<String>();
    let document = json(app, &format!("/api/document?path={encoded}&generation=1")).await;
    assert_eq!(document["content"], NOTE);
    assert_eq!(before, fs::read(vault.path().join(source)).unwrap());
    assert!(!vault.path().join(".workbench").exists());
}

#[tokio::test]
async fn refresh_is_explicit_and_stale_sources_are_rejected() {
    let (vault, _assets, app) = fixture();
    fs::write(
        vault.path().join("项目/测试项目/00-总览/INDEX.md"),
        NOTE.replace("M0", "M1"),
    )
    .unwrap();
    assert_eq!(json(app.clone(), "/api/workbench").await["generation"], 1);
    assert_eq!(
        json(app.clone(), "/api/workbench?refresh=true").await["generation"],
        2
    );
    let response = app
        .oneshot(
            request("/api/document?path=a.md&generation=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn invalid_refresh_retains_last_complete_generation() {
    let (vault, _assets, app) = fixture();
    fs::write(vault.path().join("invalid.md"), [0xff, 0xfe]).unwrap();
    let response = app
        .clone()
        .oneshot(
            request("/api/workbench?refresh=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json(app, "/api/workbench").await["generation"], 1);
}

#[tokio::test]
async fn cross_origin_rebinding_and_mutations_are_rejected() {
    let (_vault, _assets, app) = fixture();
    for (header, value) in [
        ("origin", "https://evil.example"),
        ("origin", "null"),
        ("sec-fetch-site", "cross-site"),
    ] {
        let response = app
            .clone()
            .oneshot(
                request("/api/workbench")
                    .header(header, value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/workbench")
                .header("host", "evil.example:4317")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    for method in ["POST", "PUT", "PATCH", "DELETE", "OPTIONS"] {
        let response = app
            .clone()
            .oneshot(
                request("/api/workbench")
                    .method(method)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}

#[tokio::test]
async fn static_allowlist_and_security_headers_are_enforced() {
    let (_vault, _assets, app) = fixture();
    let response = app
        .clone()
        .oneshot(request("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-origin")
    );
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    for path in [
        "/.env",
        "/assets/../index.html",
        "/assets/%2e%2e/secret.js",
        "/assets/../favicon-fixture.svg",
        "/assets/%2e%2e/favicon-fixture.svg",
        "/favicon-fixture.svg",
        "/assets/private.svg",
        "/api/attachment?path=assets/private.svg&generation=1",
        "/api/document?path=assets/private.svg&generation=1",
        "/api/missing",
        "/README.md",
    ] {
        let response = app
            .clone()
            .oneshot(request(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
    let response = app
        .clone()
        .oneshot(request("/assets/app.js").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .oneshot(
            request("/assets/favicon-fixture.svg")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/svg+xml");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap(),
        ICON.as_bytes()
    );
}
