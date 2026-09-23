//! 静态资源缓存契约：只有 `/assets/` 下带 Vite 内容哈希、且成功送出的 JS/CSS 本体长期缓存；
//! index.html（每次带不同 CSP nonce）、API、错误与其余资源一律 no-store。夹具均为临时虚构文件。

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use std::{collections::HashSet, fs};
use studiolo_local_server::router;
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;
use tower::ServiceExt;

const HOST: &str = "127.0.0.1:4317";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NOTE: &str = "---\nlifecycle: 活跃\n---\n# 缓存项目\n";
/// 无 nonce 响应的完整 CSP，逐字锁定；长缓存不得放宽任何安全头。
const STRICT_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
    connect-src 'self' http://127.0.0.1:*; img-src 'self'; font-src 'self'; base-uri 'none'; \
    form-action 'none'; frame-ancestors 'none'; object-src 'none'";

/// 形如真实 Vite 产物的哈希文件：哈希段可含 `-` 与 `_`，也可位于子目录。
const HASHED: [&str; 5] = [
    "assets/index-C65IIHot.js",
    "assets/index-ovxLtHW2.css",
    "assets/MarkdownEditor-CC7-Ql5Q.js",
    "assets/RecentlyDeleted-B_pY3tRi.js",
    "assets/chunks/vendor-Abcdefgh12.js",
];

/// 同在 `/assets/` 下，却没有哈希、哈希段不足 8 位或不紧贴扩展名、或不是脚本与样式的文件。
const UNHASHED: [&str; 6] = [
    "assets/app.js",
    "assets/app.css",
    "assets/favicon-fixture.js",
    "assets/vendor-Abcdefgh.min.js",
    "assets/favicon-4NhN4y5N.svg",
    "assets/font-Abcdefgh.woff2",
];

fn fixture() -> (TempDir, TempDir, Router) {
    let vault = TempDir::new().unwrap();
    let web = TempDir::new().unwrap();
    fs::create_dir_all(vault.path().join("项目/缓存项目/00-总览")).unwrap();
    fs::write(vault.path().join("项目/缓存项目/00-总览/INDEX.md"), NOTE).unwrap();
    fs::create_dir_all(web.path().join("assets/chunks")).unwrap();
    fs::write(
        web.path().join("index.html"),
        "<!doctype html><title>缓存</title>",
    )
    .unwrap();
    for name in HASHED.iter().chain(&UNHASHED) {
        fs::write(web.path().join(name), "/* fixture */").unwrap();
    }
    let app = router(
        ReadOnlyVault::open(vault.path()).unwrap(),
        ReadOnlyVault::open(web.path()).unwrap(),
        "缓存副本".into(),
        4317,
    )
    .unwrap();
    (vault, web, app)
}

async fn send(app: &Router, method: &str, uri: &str, host: &str) -> Response {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", host)
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

fn cache_control(response: &Response) -> &str {
    response.headers()["cache-control"].to_str().unwrap()
}

#[tokio::test]
async fn hashed_scripts_and_styles_are_immutable_and_keep_every_security_header() {
    let (_vault, _web, app) = fixture();
    for name in HASHED {
        let response = send(&app, "GET", &format!("/{name}"), HOST).await;
        assert_eq!(response.status(), StatusCode::OK, "{name}");
        assert_eq!(cache_control(&response), IMMUTABLE, "{name}");
        let headers = response.headers();
        assert_eq!(headers["x-content-type-options"], "nosniff", "{name}");
        assert_eq!(headers["referrer-policy"], "no-referrer", "{name}");
        assert_eq!(headers["cross-origin-resource-policy"], "same-origin");
        assert_eq!(headers["content-security-policy"], STRICT_CSP, "{name}");
    }
}

#[tokio::test]
async fn nonce_templated_index_is_never_cached_and_gets_a_fresh_policy_each_time() {
    let (_vault, web, app) = fixture();
    fs::write(
        web.path().join("index.html"),
        "<!doctype html><style nonce=\"__STUDIOLO_STYLE_NONCE__\"></style>",
    )
    .unwrap();
    let prefix = "default-src 'none'; script-src 'self'; style-src 'self' 'nonce-";
    let mut policies = HashSet::new();
    for _ in 0..2 {
        let response = send(&app, "GET", "/", HOST).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(cache_control(&response), "no-store");
        let csp = response.headers()["content-security-policy"].to_str();
        let csp = csp.unwrap().to_owned();
        assert!(csp.starts_with(prefix), "{csp}");
        assert!(csp.ends_with("frame-ancestors 'none'; object-src 'none'"));
        policies.insert(csp);
    }
    assert_eq!(policies.len(), 2, "每次响应都应换新 nonce");
}

#[tokio::test]
async fn index_html_api_and_unhashed_assets_stay_no_store() {
    let (_vault, _web, app) = fixture();
    let unhashed = UNHASHED.iter().map(|name| format!("/{name}"));
    for uri in ["/".to_owned(), "/api/workbench".to_owned()]
        .into_iter()
        .chain(unhashed)
    {
        let response = send(&app, "GET", &uri, HOST).await;
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        assert_eq!(cache_control(&response), "no-store", "{uri}");
    }
}

#[tokio::test]
async fn failures_on_hashed_looking_paths_are_never_cached() {
    let (_vault, _web, app) = fixture();
    for uri in [
        "/assets/index-Missing0.js",
        "/assets/.hidden-C65IIHot.js",
        "/assets/../index-C65IIHot.js",
        "/api/index-C65IIHot.js",
    ] {
        let response = send(&app, "GET", uri, HOST).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(cache_control(&response), "no-store", "{uri}");
    }
    // 目标文件确实存在，但同源检查或只读方法检查先失败：错误体同样不可缓存。
    let hashed = "/assets/index-C65IIHot.js";
    let forbidden = send(&app, "GET", hashed, "attacker.example").await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    assert_eq!(cache_control(&forbidden), "no-store");
    let write = send(&app, "POST", hashed, HOST).await;
    assert_eq!(write.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(cache_control(&write), "no-store");
}
