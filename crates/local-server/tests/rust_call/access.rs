//! 源码证据保持原认证/来源/参数门禁，HTTP 不能启用白名单或读取正文。

use axum::http::StatusCode;

use crate::{
    fixture::{PROJECT, get, request, uri},
    rust, test_support,
};

#[tokio::test]
async fn request_parameters_and_foreign_origins_cannot_expand_source_access() {
    let f = rust::prepared();
    for query in [
        "&rust-call-sites=true",
        "&files=src/api.rs",
        "&path=/tmp/secret.rs",
    ] {
        let (status, denied) = get(rust::app(&f), &format!("{}{query}", uri(PROJECT, 1))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(denied.get("rust_call_sites").is_none());
    }
    let (status, denied) = request(
        rust::app(&f),
        &uri(PROJECT, 1),
        "GET",
        "https://foreign.example",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(denied.get("rust_call_sites").is_none());
    assert_eq!(
        get(rust::app(&f), "/api/rust-source?path=src/api.rs")
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn access_authentication_precedes_source_observation() {
    let f = rust::prepared();
    let app = studiolo_local_server::router_with_access(
        studiolo_vault_broker::ReadOnlyVault::open(f.vault.path()).unwrap(),
        studiolo_vault_broker::ReadOnlyVault::open(f.assets.path()).unwrap(),
        "虚构Rust".into(),
        4320,
        test_support::verifier().await,
    )
    .unwrap();
    let app = rust::bind(&f, app, true);
    let endpoint = uri(PROJECT, 1);
    let origin = "https://studiolo.example.com";
    let (status, denied) = request(app.clone(), &endpoint, "GET", origin, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(denied.get("rust_call_sites").is_none());
    let token = test_support::signed(&test_support::claims());
    let (status, allowed) = request(app, &endpoint, "GET", origin, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(allowed["rust_call_sites"]["status"], "ready");
}
