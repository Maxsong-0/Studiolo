//! 关联字段不能绕过原认证、来源、项目、代次及参数门禁。

use axum::http::StatusCode;

use crate::{
    fixture::{PROJECT, get, request, uri},
    links::{prepared, ready},
    rust, test_support,
};

#[tokio::test]
async fn association_requests_preserve_parameter_origin_generation_and_project_gates() {
    let f = prepared();
    let app = rust::app(&f);
    for parameter in [
        "&links=true",
        "&batch_id=old",
        "&target=src/api.rs",
        "&path=outside.rs",
    ] {
        let (status, denied) = get(app.clone(), &format!("{}{parameter}", uri(PROJECT, 1))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(denied.get("rust_call_sites").is_none());
    }
    let (status, denied) = request(
        app.clone(),
        &uri(PROJECT, 1),
        "GET",
        "https://foreign.example",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(denied.get("rust_call_sites").is_none());
    assert_eq!(
        get(app.clone(), &uri("项目/不存在", 1)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(app.clone(), "/api/workbench?refresh=true").await.0,
        StatusCode::OK
    );
    let (status, stale) = get(app.clone(), &uri(PROJECT, 1)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(stale.get("rust_call_sites").is_none());
    ready(&get(app, &uri(PROJECT, 2)).await.1);
}

#[tokio::test]
async fn access_authentication_still_precedes_all_link_evidence() {
    let f = prepared();
    let app = studiolo_local_server::router_with_access(
        studiolo_vault_broker::ReadOnlyVault::open(f.vault.path()).unwrap(),
        studiolo_vault_broker::ReadOnlyVault::open(f.assets.path()).unwrap(),
        "虚构关联".into(),
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
    ready(&allowed);
}
