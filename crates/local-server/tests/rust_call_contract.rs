//! 显式源码白名单的 HTTP 契约；来源位置、代次与各类仓库证据保持独立。

#[path = "rust_call/access.rs"]
mod access;
#[path = "cargo/fixture.rs"]
#[allow(dead_code)]
mod cargo;
#[path = "project_operations/fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "rust_call/support.rs"]
mod rust;
#[path = "access/test_support.rs"]
mod test_support;

use std::fs;

use axum::http::StatusCode;
use fixture::{PROJECT, drift, get, uri};
use rust::{API, app, empty, prepared};

#[tokio::test]
async fn directory_and_cargo_bindings_never_implicitly_enable_source_reads() {
    let f = prepared();
    cargo::write(&f, "src/api.rs", "MALFORMED_PRIVATE_SOURCE");
    for server in [f.app(true), cargo::app(&f)] {
        let (status, value) = get(server, &uri(PROJECT, 1)).await;
        assert_eq!(status, StatusCode::OK);
        empty(&value, "disabled");
        assert_eq!(value["repository"]["status"], "ready");
        assert_eq!(value["architecture"]["status"], "ready");
        assert!(!value.to_string().contains("MALFORMED_PRIVATE_SOURCE"));
    }
    let (_, value) = get(f.app(false), &uri(PROJECT, 1)).await;
    empty(&value, "unbound");
}

#[tokio::test]
async fn enabled_sources_retain_all_call_kinds_and_exact_locations_without_arguments() {
    let f = prepared();
    let (status, value) = get(app(&f), &uri(PROJECT, 1)).await;
    assert_eq!(status, StatusCode::OK);
    let observation = &value["rust_call_sites"];
    assert_eq!(observation["status"], "ready");
    assert!(
        observation["observed_at"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    let graph = &observation["graph"];
    let files = graph["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["path"], "src/api.rs");
    assert_eq!(files[0]["bytes"], API.len());
    assert_eq!(files[0]["external_modules"], 1);
    assert_eq!(files[0]["unexpanded_macros"], 1);
    let calls = graph["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 4);
    for (call, (kind, name, line)) in calls.iter().zip([
        ("path", Some("helper"), 3),
        ("method", Some("send"), 4),
        ("expression", None, 5),
        ("macro", Some("include"), 6),
    ]) {
        assert_eq!(call["kind"], kind);
        assert_eq!(call["name"].as_str(), name);
        assert_eq!(call["source"]["path"], "src/api.rs");
        assert_eq!(call["source"]["line"], line);
        assert_eq!(call["source"]["column"], 5);
        assert!(
            graph["contexts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|context| context["id"] == call["context"])
        );
    }
    for secret in [
        "ARGUMENT_SECRET",
        "IGNORED_MODULE_SECRET",
        "INCLUDED_SOURCE_SECRET",
        f.repository.path().to_str().unwrap(),
    ] {
        assert!(!value.to_string().contains(secret));
    }
    assert_eq!(value["cargo_dependencies"]["status"], "ready");
    assert_eq!(value["repository"]["status"], "ready");
    assert_eq!(value["architecture"]["status"], "ready");
    assert!(!f.repository.path().join("target").exists());
}

#[tokio::test]
async fn malformed_oversized_or_missing_sources_do_not_erase_other_evidence() {
    let f = prepared();
    for (body, expected) in [
        ("fn SECRET_SYNTAX( {".to_owned(), "invalid"),
        (
            format!("fn deep() {{ {}0{}; }}", "(".repeat(80), ")".repeat(80)),
            "unsupported",
        ),
        (format!("//{}", "x".repeat(64 * 1024)), "unavailable"),
    ] {
        cargo::write(&f, "src/api.rs", &body);
        let (_, value) = get(app(&f), &uri(PROJECT, 1)).await;
        empty(&value, expected);
        assert_eq!(value["cargo_dependencies"]["status"], "ready");
        assert_eq!(value["repository"]["status"], "ready");
        assert_eq!(value["architecture"]["status"], "ready");
        assert_eq!(drift(&value, "src/model").as_deref(), Some("present"));
        assert!(!value.to_string().contains("SECRET_SYNTAX"));
    }
    fs::remove_file(f.repository.path().join("src/api.rs")).unwrap();
    let (_, value) = get(app(&f), &uri(PROJECT, 1)).await;
    empty(&value, "unavailable");
}

#[tokio::test]
async fn rust_read_permission_and_evidence_remain_independent_of_cargo_and_vault_declarations() {
    let f = prepared();
    let (_, value) = get(rust::bind(&f, f.app(false), false), &uri(PROJECT, 1)).await;
    assert_eq!(value["rust_call_sites"]["status"], "ready");
    assert_eq!(value["cargo_dependencies"]["status"], "disabled");
    cargo::write(&f, "Cargo.toml", "malformed");
    f.write_declaration("# 没有目录声明\n");
    fs::write(
        f.vault.path().join("项目/虚构/01-框架/项目运作.md"),
        "# 没有架构表\n",
    )
    .unwrap();
    let (_, value) = get(app(&f), &uri(PROJECT, 1)).await;
    assert_eq!(value["rust_call_sites"]["status"], "ready");
    assert_eq!(value["cargo_dependencies"]["status"], "invalid");
    assert_eq!(value["repository"]["status"], "unavailable");
    assert_ne!(value["architecture"]["status"], "ready");
}

#[tokio::test]
async fn source_updates_are_live_but_vault_designs_retain_the_requested_generation() {
    let f = prepared();
    let server = app(&f);
    cargo::write(&f, "src/api.rs", &API.replace("helper(", "changed("));
    fs::write(
        f.vault.path().join("项目/虚构/01-框架/项目运作.md"),
        cargo::ARCHITECTURE.replace("人工接口设计", "下一代设计"),
    )
    .unwrap();
    let (_, value) = get(server.clone(), &uri(PROJECT, 1)).await;
    assert_eq!(
        value["rust_call_sites"]["graph"]["calls"][0]["name"],
        "changed"
    );
    assert_eq!(value["architecture"]["nodes"][0]["label"], "人工接口设计");
    assert_eq!(
        get(server.clone(), "/api/workbench?refresh=true").await.0,
        StatusCode::OK
    );
    assert_eq!(
        get(server.clone(), &uri(PROJECT, 1)).await.0,
        StatusCode::CONFLICT
    );
    let (_, value) = get(server, &uri(PROJECT, 2)).await;
    assert_eq!(value["architecture"]["nodes"][0]["label"], "下一代设计");
    assert_eq!(value["rust_call_sites"]["status"], "ready");
}

#[tokio::test]
async fn other_and_unknown_projects_cannot_inherit_the_source_whitelist() {
    let f = prepared();
    let (status, other) = get(app(&f), &uri("项目/其他", 1)).await;
    assert_eq!(status, StatusCode::OK);
    empty(&other, "unbound");
    assert!(!other.to_string().contains("src/api.rs"));
    assert_eq!(
        get(app(&f), &uri("项目/不存在", 1)).await.0,
        StatusCode::NOT_FOUND
    );
}
