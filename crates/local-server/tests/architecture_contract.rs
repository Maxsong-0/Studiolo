//! 同代人工架构声明的独立 HTTP 契约；目录观测与设计可用性互不替代。

#[path = "architecture/fixture.rs"]
mod architecture;
#[path = "project_operations/fixture.rs"]
mod fixture;
#[path = "access/test_support.rs"]
mod test_support;

use std::fs;

use architecture::{ARCHITECTURE, SOURCE, no_graph, node, prepared, source, write};
use axum::http::StatusCode;
use fixture::{Fixture, PROJECT, drift, get, request, uri};

#[tokio::test]
async fn architecture_returns_nodes_relations_ordered_flows_and_exact_source_rows() {
    let f = prepared();
    let (status, value) = get(f.app(true), &uri(PROJECT, 1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["generation"], 1);
    let graph = &value["architecture"];
    assert_eq!(graph["status"], "ready");
    assert!(graph["diagnostics"].as_array().unwrap().is_empty());
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 4);
    for (id, label, kind, directory, line) in [
        ("api", "查询接口", "interface", Some("src/api"), 8),
        ("model", "只读模型", "module", Some("src/model"), 9),
        ("store", "内容存储", "data", None, 10),
        ("remote", "外部系统", "external", None, 11),
    ] {
        let item = node(graph, id);
        assert_eq!(item["label"], label);
        assert_eq!(item["kind"], kind);
        assert_eq!(item["directory"].as_str(), directory);
        source(&item["source"], line);
    }
    source(&graph["source"], 1);
    assert_eq!(
        node(graph, "model")["description"],
        "<script>设计文字</script>"
    );
    let relations = graph["relations"].as_array().unwrap();
    assert_eq!(relations.len(), 3);
    for (from, to, kind, label, line) in [
        ("api", "model", "dependency", "查询当前代次", 16),
        ("model", "store", "data_flow", "文档进入模型", 17),
        ("api", "remote", "dependency", "声明的外部依赖", 18),
    ] {
        let relation = relations
            .iter()
            .find(|row| row["from"] == from && row["to"] == to)
            .unwrap();
        assert_eq!(relation["kind"], kind);
        assert_eq!(relation["label"], label);
        source(&relation["source"], line);
    }
    let flows = graph["flows"].as_array().unwrap();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0]["id"], "read");
    assert_eq!(flows[0]["label"], "读取文档");
    let steps = flows[0]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3);
    for (index, (id, description)) in [
        ("api", "接收查询"),
        ("model", "派生文档视图"),
        ("store", "关联文档来源"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(steps[index]["order"], index + 1);
        assert_eq!(steps[index]["node"], id);
        assert_eq!(steps[index]["description"], description);
        source(&steps[index]["source"], 23 + index as u32);
    }
    assert_eq!(drift(&value, "src/model").as_deref(), Some("present"));
    assert_eq!(drift(&value, "src/api"), None);
    for forbidden in [
        "SOURCE_CONTENT_MUST_STAY_PRIVATE",
        f.repository.path().to_str().unwrap(),
    ] {
        assert!(!value.to_string().contains(forbidden));
    }
}

#[tokio::test]
async fn architecture_uses_the_document_generation_and_rejects_stale_or_unknown_queries() {
    let f = prepared();
    let app = f.app(true);
    write(
        &f,
        PROJECT,
        &ARCHITECTURE.replace("只读模型", "更新后的模型"),
    );
    let (_, previous) = get(app.clone(), &uri(PROJECT, 1)).await;
    assert_eq!(
        node(&previous["architecture"], "model")["label"],
        "只读模型"
    );
    assert_eq!(
        get(app.clone(), "/api/workbench?refresh=true").await.0,
        StatusCode::OK
    );
    assert_eq!(
        get(app.clone(), &uri(PROJECT, 1)).await.0,
        StatusCode::CONFLICT
    );
    let (status, current) = get(app.clone(), &uri(PROJECT, 2)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        node(&current["architecture"], "model")["label"],
        "更新后的模型"
    );
    assert_eq!(
        get(app.clone(), &uri("项目/不存在", 2)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(app, &format!("{}&source=/tmp/other.md", uri(PROJECT, 2)))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn an_unbound_project_still_shows_design_but_has_no_directory_evidence() {
    let f = prepared();
    let (status, value) = get(f.app(false), &uri(PROJECT, 1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["architecture"]["status"], "ready");
    assert_eq!(value["repository"]["status"], "unbound");
    assert!(
        value["drift"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["status"] == "unverified")
    );
}

#[tokio::test]
async fn missing_or_invalid_architecture_keeps_the_complete_directory_result() {
    let f = Fixture::new();
    let (_, missing) = get(f.app(true), &uri(PROJECT, 1)).await;
    no_graph(&missing["architecture"], "unavailable");
    assert_eq!(missing["declaration_status"], "ready");
    assert_eq!(drift(&missing, "src/model").as_deref(), Some("present"));
    for content in [
        ARCHITECTURE.replace("| api | model | 依赖 |", "| api | absent | 依赖 |"),
        ARCHITECTURE.replace("| read | 读取文档 | 2 |", "| read | 读取文档 | 4 |"),
        ARCHITECTURE.replace("## 关系", "## 其他关系"),
        format!("{ARCHITECTURE}\n{}", "x".repeat(64 * 1024)),
    ] {
        write(&f, PROJECT, &content);
        let (status, value) = get(f.app(true), &uri(PROJECT, 1)).await;
        assert_eq!(status, StatusCode::OK);
        no_graph(&value["architecture"], "invalid");
        source(&value["architecture"]["source"], 1);
        assert_eq!(value["repository"]["status"], "ready");
        assert_eq!(drift(&value, "src/model").as_deref(), Some("present"));
    }
}

#[tokio::test]
async fn an_architecture_only_project_does_not_require_a_directory_declaration() {
    let f = prepared();
    fs::remove_file(f.vault.path().join("项目/虚构/01-框架/目录结构.md")).unwrap();
    let (status, value) = get(f.app(true), &uri(PROJECT, 1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["architecture"]["status"], "ready");
    assert_eq!(value["declaration_status"], "unavailable");
    assert!(value["drift"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn explicit_empty_relation_and_flow_tables_are_valid_designs() {
    let f = prepared();
    let empty_tables = ARCHITECTURE
        .lines()
        .filter(|line| {
            ![
                "| api | model |",
                "| model | store |",
                "| api | remote |",
                "| read |",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        })
        .collect::<Vec<_>>()
        .join("\n");
    write(&f, PROJECT, &empty_tables);
    let (_, value) = get(f.app(false), &uri(PROJECT, 1)).await;
    assert_eq!(value["architecture"]["status"], "ready");
    assert_eq!(value["architecture"]["nodes"].as_array().unwrap().len(), 4);
    assert!(
        value["architecture"]["relations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        value["architecture"]["flows"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn design_directory_associations_do_not_expand_repository_observation() {
    let f = prepared();
    write(
        &f,
        PROJECT,
        &ARCHITECTURE.replace("src/api", "docs/architecture-only"),
    );
    fs::create_dir(f.repository.path().join("docs/architecture-only")).unwrap();
    std::os::unix::fs::symlink(
        f.assets.path(),
        f.repository
            .path()
            .join("docs/architecture-only/private-link"),
    )
    .unwrap();
    let (_, value) = get(f.app(true), &uri(PROJECT, 1)).await;
    assert_eq!(value["architecture"]["status"], "ready");
    assert_eq!(value["repository"]["status"], "ready");
    assert_eq!(drift(&value, "docs/architecture-only"), None);
    assert!(!value.to_string().contains("private-link"));
    std::os::unix::fs::symlink(f.assets.path(), f.repository.path().join("src/link")).unwrap();
    let (_, unavailable) = get(f.app(true), &uri(PROJECT, 1)).await;
    assert_eq!(unavailable["architecture"]["status"], "ready");
    assert_eq!(unavailable["repository"]["status"], "unavailable");
    assert!(
        unavailable["drift"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["status"] == "unverified")
    );
}

#[tokio::test]
async fn project_selection_never_imports_another_projects_architecture_or_binding() {
    let f = prepared();
    let other = ARCHITECTURE.replace("只读模型", "OTHER_PROJECT_ONLY");
    write(&f, "项目/其他", &other);
    let (_, first) = get(f.app(true), &uri(PROJECT, 1)).await;
    assert!(!first.to_string().contains("OTHER_PROJECT_ONLY"));
    let (_, second) = get(f.app(true), &uri("项目/其他", 1)).await;
    assert_eq!(second["architecture"]["status"], "ready");
    assert_eq!(
        second["architecture"]["source"]["path"],
        "项目/其他/01-框架/项目运作.md"
    );
    assert_eq!(
        node(&second["architecture"], "model")["label"],
        "OTHER_PROJECT_ONLY"
    );
    assert_eq!(second["repository"]["status"], "unbound");
    assert!(!second.to_string().contains(SOURCE));
}

#[tokio::test]
async fn access_guard_protects_the_new_design_field_before_returning_any_graph() {
    let f = prepared();
    let app = studiolo_local_server::router_with_access(
        studiolo_vault_broker::ReadOnlyVault::open(f.vault.path()).unwrap(),
        studiolo_vault_broker::ReadOnlyVault::open(f.assets.path()).unwrap(),
        "虚构设计".into(),
        4320,
        test_support::verifier().await,
    )
    .unwrap();
    let endpoint = uri(PROJECT, 1);
    let origin = "https://studiolo.example.com";
    let (status, denied) = request(app.clone(), &endpoint, "GET", origin, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(denied.get("architecture").is_none());
    assert!(!denied.to_string().contains("只读模型"));
    let token = test_support::signed(&test_support::claims());
    let (status, allowed) = request(app, &endpoint, "GET", origin, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(allowed["architecture"]["status"], "ready");
}
