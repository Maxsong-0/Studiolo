//! 新能力继续受原查询、来源和 Access 门禁约束，不提供正文读取路由。

use axum::http::StatusCode;

use crate::{
    cargo,
    fixture::{PROJECT, get, request, uri},
    test_support,
};

#[tokio::test]
async fn request_parameters_cannot_enable_cargo_or_select_an_arbitrary_manifest() {
    let f = cargo::prepared();
    for query in [
        "&cargo-dependencies=true",
        "&manifest=/tmp/secret",
        "&path=Cargo.toml",
    ] {
        let (status, denied) = get(cargo::app(&f), &format!("{}{query}", uri(PROJECT, 1))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(denied.get("cargo_dependencies").is_none());
    }
    let (status, denied) = request(
        cargo::app(&f),
        &uri(PROJECT, 1),
        "GET",
        "https://foreign.example",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(denied.get("cargo_dependencies").is_none());
    assert_eq!(
        get(cargo::app(&f), "/api/cargo-manifest?path=Cargo.toml")
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn access_authentication_precedes_cargo_manifest_observation() {
    let f = cargo::prepared();
    let app = studiolo_local_server::router_with_access(
        studiolo_vault_broker::ReadOnlyVault::open(f.vault.path()).unwrap(),
        studiolo_vault_broker::ReadOnlyVault::open(f.assets.path()).unwrap(),
        "虚构Cargo".into(),
        4320,
        test_support::verifier().await,
    )
    .unwrap();
    let app = cargo::bind(&f, app);
    let endpoint = uri(PROJECT, 1);
    let origin = "https://studiolo.example.com";
    let (status, denied) = request(app.clone(), &endpoint, "GET", origin, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(denied.get("cargo_dependencies").is_none());
    let token = test_support::signed(&test_support::claims());
    let (status, allowed) = request(app, &endpoint, "GET", origin, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(allowed["cargo_dependencies"]["status"], "ready");
}
