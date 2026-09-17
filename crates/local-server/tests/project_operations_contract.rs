//! 真实只读目录 → 同代声明 → HTTP 的图2首片契约。

#[path = "project_operations/fixture.rs"]
mod fixture;
#[path = "project_operations/process.rs"]
mod process;
#[path = "access/test_support.rs"]
mod test_support;

use axum::http::StatusCode;
use fixture::{DECLARATION, Fixture, PROJECT, drift, get, request, uri};
use std::fs;

#[tokio::test]
async fn explicit_binding_reports_both_directions_and_never_exposes_source_bytes() {
    let f = Fixture::new();
    let (status, value) = get(f.app(true), &uri(PROJECT, 1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["declaration_status"], "ready");
    assert_eq!(value["repository"]["status"], "ready");
    assert!(
        value["repository"]["observed_at"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    assert_eq!(drift(&value, "src/model").as_deref(), Some("present"));
    assert_eq!(drift(&value, "src/missing").as_deref(), Some("missing"));
    assert_eq!(drift(&value, "src/extra").as_deref(), Some("undocumented"));
    assert_eq!(drift(&value, "src/model/internal"), None);
    let json = value.to_string();
    for forbidden in [
        "SOURCE_CONTENT_MUST_STAY_PRIVATE",
        "机密档案",
        "private/",
        ".hidden",
        f.repository.path().to_str().unwrap(),
    ] {
        assert!(!json.contains(forbidden));
    }
    assert!(json.contains("不得执行"));
}

#[tokio::test]
async fn unbound_and_other_projects_cannot_inherit_the_repository_capability() {
    let f = Fixture::new();
    let (_, value) = get(f.app(false), &uri(PROJECT, 1)).await;
    assert_eq!(value["repository"]["status"], "unbound");
    assert!(
        value["drift"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["status"] == "unverified")
    );
    let (_, other) = get(f.app(true), &uri("项目/其他", 1)).await;
    assert_eq!(other["repository"]["status"], "unbound");
    assert_eq!(other["declaration_status"], "unavailable");
    assert!(other["drift"].as_array().unwrap().is_empty());
    assert_eq!(
        get(f.app(true), &uri("项目/不存在", 1)).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn repository_changes_are_observed_again_but_documents_require_generation_refresh() {
    let f = Fixture::new();
    let app = f.app(true);
    fs::rename(
        f.repository.path().join("src/extra"),
        f.repository.path().join("src/missing"),
    )
    .unwrap();
    let (_, changed) = get(app.clone(), &uri(PROJECT, 1)).await;
    assert_eq!(drift(&changed, "src/missing").as_deref(), Some("present"));
    assert_eq!(drift(&changed, "src/extra"), None);
    f.write_declaration(&DECLARATION.replace("missing/", "renamed/"));
    let (_, before) = get(app.clone(), &uri(PROJECT, 1)).await;
    assert_eq!(drift(&before, "src/renamed"), None);
    assert_eq!(
        get(app.clone(), "/api/workbench?refresh=true").await.0,
        StatusCode::OK
    );
    assert_eq!(
        get(app.clone(), &uri(PROJECT, 1)).await.0,
        StatusCode::CONFLICT
    );
    let (_, after) = get(app, &uri(PROJECT, 2)).await;
    assert_eq!(drift(&after, "src/renamed").as_deref(), Some("missing"));
    assert_eq!(
        drift(&after, "src/missing").as_deref(),
        Some("undocumented")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn unsafe_or_replaced_repository_loses_all_drift_evidence() {
    let f = Fixture::new();
    let app = f.app(true);
    std::os::unix::fs::symlink(f.assets.path(), f.repository.path().join("src/link")).unwrap();
    let (_, unsafe_value) = get(app.clone(), &uri(PROJECT, 1)).await;
    assert_eq!(unsafe_value["repository"]["status"], "unavailable");
    assert!(
        unsafe_value["drift"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["status"] == "unverified")
    );
    fs::remove_file(f.repository.path().join("src/link")).unwrap();
    let old = f.repository.path().join("original-root");
    let moved = f.assets.path().join("moved-repository");
    fs::rename(f.repository.path(), &moved).unwrap();
    fs::create_dir(f.repository.path()).unwrap();
    fs::create_dir(&old).unwrap();
    let (_, value) = get(app, &uri(PROJECT, 1)).await;
    assert_eq!(value["repository"]["status"], "unavailable");
    assert!(value["repository"]["observed_at"].is_null());
    assert!(!value.to_string().contains("original-root"));
}

#[tokio::test]
async fn missing_parent_is_missing_and_invalid_declaration_is_never_a_clean_report() {
    let f = Fixture::new();
    fs::remove_dir_all(f.repository.path().join("src")).unwrap();
    let (_, value) = get(f.app(true), &uri(PROJECT, 1)).await;
    assert_eq!(value["repository"]["status"], "ready");
    assert_eq!(drift(&value, "src/model").as_deref(), Some("missing"));
    f.write_declaration(&DECLARATION.replace("src/ #", "../src/ #"));
    let (_, invalid) = get(f.app(true), &uri(PROJECT, 1)).await;
    assert_eq!(invalid["declaration_status"], "invalid");
    assert_ne!(invalid["repository"]["status"], "ready");
}

#[tokio::test]
async fn generation_query_method_and_origin_boundaries_remain_enforced() {
    let f = Fixture::new();
    for endpoint in [
        "/api/project-operations",
        "/api/project-operations?project=a",
        "/api/project-operations?project=a&generation=1&root=/tmp",
        "/api/project-operations?project=a&project=b&generation=1",
    ] {
        assert_eq!(get(f.app(true), endpoint).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        request(
            f.app(true),
            &uri(PROJECT, 1),
            "POST",
            "http://127.0.0.1:4320",
            None
        )
        .await
        .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        request(
            f.app(true),
            &uri(PROJECT, 1),
            "GET",
            "https://foreign.example",
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn access_authentication_still_guards_bound_repository_directory_names() {
    let f = Fixture::new();
    let app = studiolo_local_server::router_with_access(
        studiolo_vault_broker::ReadOnlyVault::open(f.vault.path()).unwrap(),
        studiolo_vault_broker::ReadOnlyVault::open(f.assets.path()).unwrap(),
        "虚构".into(),
        4320,
        test_support::verifier().await,
    )
    .unwrap();
    let app = f.bind(app);
    let endpoint = uri(PROJECT, 1);
    let origin = "https://studiolo.example.com";
    assert_eq!(
        request(app.clone(), &endpoint, "GET", origin, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(app.clone(), &endpoint, "GET", origin, Some("invalid"))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let token = test_support::signed(&test_support::claims());
    assert_eq!(
        request(app, &endpoint, "GET", origin, Some(&token)).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn aggregate_directory_budget_failure_is_unverified_not_a_partial_report() {
    let f = Fixture::new();
    for index in 0..1025 {
        fs::create_dir(f.repository.path().join(format!("dir-{index:04}"))).unwrap();
    }
    let (_, value) = get(f.app(true), &uri(PROJECT, 1)).await;
    assert_eq!(value["repository"]["status"], "unavailable");
    assert!(value["repository"]["observed_at"].is_null());
    assert!(!value.to_string().contains("dir-0000"));
    assert!(
        value["drift"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["status"] == "unverified")
    );
}
