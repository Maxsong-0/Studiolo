//! 同批调用关联的独立 HTTP 契约；复用已存在的认证与只读仓库夹具。

#[path = "rust_call_links/access.rs"]
mod access;
#[path = "cargo/fixture.rs"]
#[allow(dead_code)]
mod cargo;
#[path = "project_operations/fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "rust_call_links/support.rs"]
mod links;
#[path = "rust_call/support.rs"]
#[allow(dead_code)]
mod rust;
#[path = "access/test_support.rs"]
mod test_support;

use std::fs;

use axum::http::StatusCode;
use fixture::{PROJECT, get, uri};
use links::{API, empty, prepared, ready};

#[tokio::test]
async fn ready_publishes_same_file_function_targets_and_reverse_references_without_source_bodies() {
    let f = prepared();
    let (status, value) = get(rust::app(&f), &uri(PROJECT, 1)).await;
    assert_eq!(status, StatusCode::OK);
    let links = ready(&value);
    let rows = links["associations"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.iter().filter(|row| row["status"] == "linked").count(),
        3
    );
    assert_eq!(rows[0]["target"], rows[1]["target"]);
    assert_eq!(rows[2]["reason"], "not_found");
    assert_ne!(rows[0]["target"], rows[3]["target"]);
    assert_eq!(value["cargo_dependencies"]["status"], "ready");
    assert_eq!(value["repository"]["status"], "ready");
    assert_eq!(value["architecture"]["status"], "ready");
    for secret in [
        "ARGUMENT_SECRET",
        "IGNORED_MODULE_SECRET",
        "INCLUDED_SOURCE_SECRET",
        f.repository.path().to_str().unwrap(),
    ] {
        assert!(!value.to_string().contains(secret));
    }
    assert!(!f.repository.path().join("target").exists());
}

#[tokio::test]
async fn disabled_and_unbound_publish_explicit_null_links_without_reading_bad_sources() {
    let f = prepared();
    cargo::write(&f, "src/api.rs", "MALFORMED_PRIVATE_SOURCE");
    for app in [f.app(true), cargo::app(&f)] {
        let (status, value) = get(app, &uri(PROJECT, 1)).await;
        assert_eq!(status, StatusCode::OK);
        empty(&value, "disabled");
        assert!(!value.to_string().contains("MALFORMED_PRIVATE_SOURCE"));
    }
    let (_, value) = get(f.app(false), &uri(PROJECT, 1)).await;
    empty(&value, "unbound");
    let (_, other) = get(rust::app(&f), &uri("项目/其他", 1)).await;
    empty(&other, "unbound");
    assert!(!other.to_string().contains("src/api.rs"));
}

#[tokio::test]
async fn same_generation_and_same_location_ids_do_not_reuse_links_from_old_content() {
    let f = prepared();
    let app = rust::app(&f);
    let (_, before) = get(app.clone(), &uri(PROJECT, 1)).await;
    let old_batch = ready(&before)["batch_id"].clone();
    let changed = API.replace("helper", "worker");
    assert_eq!(changed.len(), API.len());
    cargo::write(&f, "src/api.rs", &changed);
    let (_, after) = get(app, &uri(PROJECT, 1)).await;
    let new_links = ready(&after);
    assert_eq!(before["generation"], after["generation"]);
    assert_ne!(old_batch, new_links["batch_id"]);
    let ids = |value: &serde_json::Value| {
        value["rust_call_sites"]["graph"]["calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|call| call["id"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&before), ids(&after));
    let contexts = after["rust_call_sites"]["graph"]["contexts"]
        .as_array()
        .unwrap();
    let target = contexts
        .iter()
        .find(|row| row["id"] == new_links["associations"][0]["target"])
        .unwrap();
    assert_eq!(target["name"], "worker");
}

#[tokio::test]
async fn failed_observations_clear_links_and_recovery_uses_only_current_content() {
    let f = prepared();
    let app = rust::app(&f);
    let (_, before) = get(app.clone(), &uri(PROJECT, 1)).await;
    let old_batch = ready(&before)["batch_id"].clone();
    for (body, status) in [
        ("fn SECRET_SYNTAX( {".to_owned(), "invalid"),
        (
            format!("fn deep() {{ {}0{}; }}", "(".repeat(80), ")".repeat(80)),
            "unsupported",
        ),
        (format!("//{}", "x".repeat(64 * 1024)), "unavailable"),
    ] {
        cargo::write(&f, "src/api.rs", &body);
        let (http_status, value) = get(app.clone(), &uri(PROJECT, 1)).await;
        assert_eq!(http_status, StatusCode::OK);
        empty(&value, status);
        assert_eq!(value["cargo_dependencies"]["status"], "ready");
        assert_eq!(value["repository"]["status"], "ready");
        assert_eq!(value["architecture"]["status"], "ready");
        assert!(!value.to_string().contains("SECRET_SYNTAX"));
    }
    fs::remove_file(f.repository.path().join("src/api.rs")).unwrap();
    empty(&get(app.clone(), &uri(PROJECT, 1)).await.1, "unavailable");
    cargo::write(&f, "src/api.rs", &API.replace("helper", "worker"));
    let (_, recovered) = get(app, &uri(PROJECT, 1)).await;
    assert_ne!(old_batch, ready(&recovered)["batch_id"]);
}

#[tokio::test]
async fn replaced_repository_roots_cannot_publish_previously_ready_links() {
    let f = prepared();
    let app = rust::app(&f);
    ready(&get(app.clone(), &uri(PROJECT, 1)).await.1);
    let retained = tempfile::TempDir::new_in("/tmp").unwrap();
    fs::rename(f.repository.path(), retained.path().join("old-root")).unwrap();
    fs::create_dir(f.repository.path()).unwrap();
    let (status, value) = get(app, &uri(PROJECT, 1)).await;
    assert_eq!(status, StatusCode::OK);
    empty(&value, "unavailable");
    assert_eq!(value["architecture"]["status"], "ready");
    assert!(
        !value
            .to_string()
            .contains(retained.path().to_str().unwrap())
    );
}
