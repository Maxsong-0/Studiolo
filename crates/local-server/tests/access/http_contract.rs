//! 完整 HTTP Interface 的 Access 契约；经固定虚构验证器装配临时夹具。

use super::test_support::{claims, signed, verifier};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, request::Builder},
    response::Response,
};
use serde_json::{Value, json};
use std::fs;
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;
use tower::ServiceExt;

pub(super) const ORIGIN: &str = "https://studiolo.example.com";
pub(super) const SEARCH: &str = "/api/search?q=reading&mode=full_text&generation=1";
pub(super) const DOCUMENT: &str = "/api/document?path=a.md&generation=1";
pub(super) const TREE: &str = "/api/tree?path=&generation=1&offset=0";
pub(super) const ROUTES: [&str; 7] = [
    "/",
    "/assets/app.js",
    "/assets/app.css",
    "/api/workbench",
    SEARCH,
    DOCUMENT,
    TREE,
];

pub(super) async fn fixture(public: bool) -> (TempDir, Router) {
    let root = TempDir::new().unwrap();
    let vault = root.path().join("vault");
    let assets = root.path().join("web");
    fs::create_dir(&vault).unwrap();
    fs::create_dir_all(assets.join("assets")).unwrap();
    fs::write(vault.join("a.md"), "# 示例\n[[b#目标]]\nreading\n").unwrap();
    fs::write(vault.join("b.md"), "# 目标\n虚构内容\n").unwrap();
    for (path, content) in [
        ("index.html", "<!doctype html><title>虚构示例</title>"),
        ("assets/app.js", "console.log('fixture')"),
        ("assets/app.css", "body { color: black; }"),
    ] {
        fs::write(assets.join(path), content).unwrap();
    }
    let vault = ReadOnlyVault::open(&vault).unwrap();
    let assets = ReadOnlyVault::open(&assets).unwrap();
    let app = if public {
        studiolo_local_server::router_with_access(
            vault,
            assets,
            "虚构示例".into(),
            4319,
            verifier().await,
        )
    } else {
        studiolo_local_server::router(vault, assets, "虚构示例".into(), 4319)
    }
    .unwrap();
    (root, app)
}

pub(super) fn request(uri: &str, token: Option<&str>) -> Builder {
    let builder = Request::builder().uri(uri).header("host", "127.0.0.1:4319");
    if let Some(token) = token {
        builder.header("cf-access-jwt-assertion", token)
    } else {
        builder
    }
}

pub(super) async fn reply(app: &Router, request: Builder, status: u16) -> Response {
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), status);
    assert_security_headers(&response);
    response
}

pub(super) fn assert_security_headers(response: &Response) {
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        assert_eq!(response.headers()[name], value, "{name}");
    }
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(csp.contains("default-src 'none'"));
    assert!(csp.contains("frame-ancestors 'none'"));
    for key in response.headers().keys() {
        assert!(!key.as_str().starts_with("access-control-"));
    }
}

async fn json(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn valid_signed_identity_reads_static_snapshot_search_and_navigation() {
    let (_root, app) = fixture(true).await;
    let token = signed(&claims());
    for uri in ROUTES {
        let request = request(uri, Some(&token)).header("origin", ORIGIN);
        let response = reply(&app, request, 200).await;
        if uri == DOCUMENT {
            let document = json(response).await;
            assert_eq!(document["path"], "a.md");
            let navigation = &document["navigation"];
            assert_eq!(navigation["outgoing"][0]["status"], "resolved");
            assert_eq!(navigation["outgoing"][0]["targets"][0]["path"], "b.md");
            assert!(!navigation["outline"].as_array().unwrap().is_empty());
        } else if uri == SEARCH {
            assert_eq!(json(response).await["hits"][0]["path"], "a.md");
        } else if uri == "/api/workbench" {
            assert_eq!(json(response).await["generation"], 1);
        }
    }
    reply(&app, request("/assets/missing.js", Some(&token)), 404).await;
}

#[tokio::test]
async fn prior_valid_identity_never_replaces_verification_of_the_next_request() {
    let (_root, app) = fixture(true).await;
    let valid = signed(&claims());
    reply(&app, request("/api/workbench", Some(&valid)), 200).await;
    let mut expired = claims();
    expired["exp"] = json!(1);
    let invalid = signed(&expired);
    reply(&app, request("/api/workbench", Some(&invalid)), 401).await;
    reply(
        &app,
        request("/api/workbench", None).header("cookie", format!("CF_Authorization={valid}")),
        401,
    )
    .await;
    reply(&app, request("/api/workbench", Some(&valid)), 200).await;
}

#[tokio::test]
async fn absent_token_protects_every_route_including_unknown_paths_and_refresh() {
    let (_root, app) = fixture(true).await;
    let extra = [
        "/assets/missing.js",
        "/api/missing",
        "/api/workbench?refresh=true",
    ];
    for uri in ROUTES.into_iter().chain(extra) {
        reply(&app, request(uri, None), 401).await;
    }
    let token = signed(&claims());
    let response = reply(&app, request("/api/workbench", Some(&token)), 200).await;
    assert_eq!(json(response).await["generation"], 1);
}

#[tokio::test]
async fn unsigned_identity_cookie_and_forwarding_headers_cannot_replace_assertion() {
    let (_root, app) = fixture(true).await;
    let token = signed(&claims());
    for (name, value) in [
        (
            "cf-access-authenticated-user-email",
            "maxsong@example.com".into(),
        ),
        ("cookie", format!("CF_Authorization={token}")),
        ("authorization", format!("Bearer {token}")),
        ("x-forwarded-host", "studiolo.example.com".into()),
        ("x-forwarded-proto", "https".into()),
        ("x-forwarded-user", "maxsong@example.com".into()),
    ] {
        reply(
            &app,
            request("/api/workbench", None).header(name, value),
            401,
        )
        .await;
    }
}

#[tokio::test]
async fn rejected_signed_claims_are_unauthorized_despite_a_matching_unsigned_email() {
    let (_root, app) = fixture(true).await;
    for (field, value) in [
        ("exp", json!(1)),
        ("iss", json!("https://wrong.cloudflareaccess.com")),
        ("aud", json!(["wrong-audience"])),
        ("email", json!("other@example.com")),
        ("type", json!("service")),
        ("nbf", json!(u64::MAX)),
    ] {
        let mut payload = claims();
        payload[field] = value;
        let token = signed(&payload);
        let request = request("/api/workbench", Some(&token))
            .header("cf-access-authenticated-user-email", "maxsong@example.com");
        reply(&app, request, 401).await;
    }
}
