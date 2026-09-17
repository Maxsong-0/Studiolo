//! 固定 Cargo 清单能力的 HTTP 契约；正文、目录观测和人工架构证据保持独立。

#[path = "cargo/access.rs"]
mod access;
#[path = "cargo/fixture.rs"]
mod cargo;
#[path = "project_operations/fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "access/test_support.rs"]
mod test_support;

use std::fs;

use axum::http::StatusCode;
use cargo::{API, ARCHITECTURE, app, empty, prepared, write};
use fixture::{PROJECT, drift, get, uri};

#[tokio::test]
async fn original_directory_binding_stays_disabled_and_absent_binding_is_unbound() {
    let f = prepared();
    write(&f, "Cargo.toml", "MALFORMED_PRIVATE_CONTENT");
    let (status, value) = get(f.app(true), &uri(PROJECT, 1)).await;
    assert_eq!(status, StatusCode::OK);
    empty(&value, "disabled");
    assert_eq!(value["repository"]["status"], "ready");
    assert_eq!(value["architecture"]["status"], "ready");
    assert_eq!(drift(&value, "src/model").as_deref(), Some("present"));
    assert!(!value.to_string().contains("MALFORMED_PRIVATE_CONTENT"));
    let (_, unbound) = get(f.app(false), &uri(PROJECT, 1)).await;
    empty(&unbound, "unbound");
}

#[tokio::test]
async fn enabled_cargo_observation_has_exact_manifest_sources_and_no_raw_content() {
    let f = prepared();
    let (status, value) = get(app(&f), &uri(PROJECT, 1)).await;
    assert_eq!(status, StatusCode::OK);
    let cargo = &value["cargo_dependencies"];
    assert_eq!(cargo["status"], "ready");
    assert!(
        cargo["observed_at"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    let graph = &cargo["graph"];
    assert_eq!(graph["packages"].as_array().unwrap().len(), 2);
    assert_eq!(graph["packages"][0]["id"], "crates/api");
    assert_eq!(graph["packages"][0]["name"], "api");
    assert_eq!(
        graph["packages"][0]["source"]["path"],
        "crates/api/Cargo.toml"
    );
    assert_eq!(graph["packages"][0]["source"]["line"], 2);
    assert_eq!(graph["external_dependencies"], 1);
    let dependencies = graph["dependencies"].as_array().unwrap();
    assert_eq!(dependencies.len(), 1);
    let edge = &dependencies[0];
    assert_eq!(edge["from"], "crates/api");
    assert_eq!(edge["to"], "crates/model");
    assert_eq!(edge["alias"], "model");
    assert_eq!(edge["kind"], "normal");
    assert_eq!(edge["optional"], false);
    assert!(edge["target"].is_null());
    assert_eq!(edge["source"]["path"], "crates/api/Cargo.toml");
    assert_eq!(edge["source"]["line"], 5);
    assert_eq!(edge["inherited_from"]["path"], "Cargo.toml");
    assert_eq!(edge["inherited_from"]["line"], 4);
    for forbidden in [
        "SOURCE_SECRET",
        "LOCK_CONTENT_MUST_STAY_PRIVATE",
        "SOURCE_CONTENT_MUST_STAY_PRIVATE",
        f.repository.path().to_str().unwrap(),
    ] {
        assert!(!value.to_string().contains(forbidden));
    }
    assert!(!f.repository.path().join("target").exists());
    assert!(!f.repository.path().join(".workbench").exists());
}

#[tokio::test]
async fn invalid_unsupported_and_missing_manifests_do_not_erase_other_evidence() {
    let f = prepared();
    for (content, expected) in [
        ("[workspace]\nmembers = [\"crates/api\"\n", "invalid"),
        ("[workspace]\nmembers = [\"crates/*\"]\n", "unsupported"),
    ] {
        write(&f, "Cargo.toml", content);
        let (_, value) = get(app(&f), &uri(PROJECT, 1)).await;
        empty(&value, expected);
        assert_eq!(value["architecture"]["status"], "ready");
        assert_eq!(value["repository"]["status"], "ready");
        assert_eq!(drift(&value, "src/model").as_deref(), Some("present"));
    }
    fs::remove_file(f.repository.path().join("Cargo.toml")).unwrap();
    let (_, missing) = get(app(&f), &uri(PROJECT, 1)).await;
    empty(&missing, "unavailable");
    assert_eq!(missing["architecture"]["status"], "ready");
    assert_eq!(missing["repository"]["status"], "ready");
}

#[tokio::test]
async fn repository_manifest_updates_are_live_while_design_documents_keep_the_generation() {
    let f = prepared();
    let server = app(&f);
    write(
        &f,
        "crates/api/Cargo.toml",
        &API.replace("\"api\"", "\"api-v2\""),
    );
    fs::write(
        f.vault.path().join("项目/虚构/01-框架/项目运作.md"),
        ARCHITECTURE.replace("人工接口设计", "下一代设计"),
    )
    .unwrap();
    let (_, current) = get(server.clone(), &uri(PROJECT, 1)).await;
    assert_eq!(
        current["cargo_dependencies"]["graph"]["packages"][0]["name"],
        "api-v2"
    );
    assert_eq!(current["architecture"]["nodes"][0]["label"], "人工接口设计");
    assert_eq!(
        get(server.clone(), "/api/workbench?refresh=true").await.0,
        StatusCode::OK
    );
    assert_eq!(
        get(server.clone(), &uri(PROJECT, 1)).await.0,
        StatusCode::CONFLICT
    );
    let (_, refreshed) = get(server, &uri(PROJECT, 2)).await;
    assert_eq!(refreshed["architecture"]["nodes"][0]["label"], "下一代设计");
    assert_eq!(refreshed["cargo_dependencies"]["status"], "ready");
}

#[tokio::test]
async fn a_different_project_cannot_inherit_the_cargo_read_capability() {
    let f = prepared();
    let (status, other) = get(app(&f), &uri("项目/其他", 1)).await;
    assert_eq!(status, StatusCode::OK);
    empty(&other, "unbound");
    assert!(!other.to_string().contains("crates/api"));
    assert_eq!(
        get(app(&f), &uri("项目/不存在", 1)).await.0,
        StatusCode::NOT_FOUND
    );
}
