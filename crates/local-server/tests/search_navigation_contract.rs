//! 搜索与导航从真实只读临时文件装配，不借用用户笔记。

use std::fs;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::Value;
use studiolo_local_server::router;
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;
use tower::ServiceExt;

const SOURCE: &str = "项目/甲/00-总览/INDEX.md";
const TARGET: &str = "项目/甲/01-框架/中文说明.md";
const TEXT: &str = "# 中文说明\r\n\r\n## 阅读节奏\r\n连续阅读，保持段落节奏。\r\n正文 <script>throw new Error('never')</script>\r\n";

fn fixture() -> (TempDir, TempDir, Router) {
    let vault = TempDir::new().unwrap();
    let assets = TempDir::new().unwrap();
    for (path, content) in [
        (
            SOURCE,
            "# 甲\n[[中文说明#阅读节奏|去阅读]]\n[[缺失文档]]\n[[中文说明#不存在]]\n",
        ),
        (TARGET, TEXT),
        (
            "项目/乙/中文说明.md",
            "# 乙的说明\n连续阅读，另一个项目。\n",
        ),
    ] {
        let path = vault.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    fs::write(
        assets.path().join("index.html"),
        "<!doctype html><title>搜索验收</title>",
    )
    .unwrap();
    let app = router(
        ReadOnlyVault::open(vault.path()).unwrap(),
        ReadOnlyVault::open(assets.path()).unwrap(),
        "搜索验收夹具".into(),
        4317,
    )
    .unwrap();
    (vault, assets, app)
}

fn encoded(value: &str) -> String {
    value.bytes().map(|byte| format!("%{byte:02X}")).collect()
}

fn search_uri(query: &str, mode: &str, project: Option<&str>, generation: u64) -> String {
    let project = project
        .map(|project| format!("&project={}", encoded(project)))
        .unwrap_or_default();
    format!(
        "/api/search?q={}&mode={mode}&generation={generation}{project}",
        encoded(query)
    )
}

async fn response(app: Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", "127.0.0.1:4317")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn chinese_fulltext_scope_filename_and_source_line_share_one_generation() {
    let (_vault, _assets, app) = fixture();
    let (status, all) = response(app.clone(), &search_uri("连续阅读", "full_text", None, 1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all["generation"], 1);
    assert_eq!(all["hits"].as_array().unwrap().len(), 2);
    let (_, scoped) = response(
        app.clone(),
        &search_uri("阅读，保持", "full_text", Some("项目/甲"), 1),
    )
    .await;
    assert_eq!(scoped["hits"].as_array().unwrap().len(), 1);
    assert_eq!(scoped["hits"][0]["path"], TARGET);
    assert_eq!(scoped["hits"][0]["line"], 4);
    assert_eq!(scoped["hits"][0]["kind"], "content");
    let (_, files) = response(
        app,
        &search_uri("中文说明.md", "full_text", Some("项目/甲"), 1),
    )
    .await;
    assert_eq!(files["hits"][0]["kind"], "path");
    assert_eq!(files["hits"][0]["path"], TARGET);
}

#[tokio::test]
async fn quick_switch_uses_same_search_route_with_full_path_fragments() {
    let (_vault, _assets, app) = fixture();
    let (status, result) = response(app, &search_uri("甲 框架 中文", "path", None, 1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["hits"].as_array().unwrap().len(), 1);
    assert_eq!(result["hits"][0]["path"], TARGET);
    assert_eq!(result["hits"][0]["kind"], "path");
}

#[tokio::test]
async fn source_includes_resolved_heading_missing_states_outline_and_backlinks() {
    let (_vault, _assets, app) = fixture();
    let (_, source) = response(
        app.clone(),
        &format!("/api/document?path={}&generation=1", encoded(SOURCE)),
    )
    .await;
    let links = source["navigation"]["outgoing"].as_array().unwrap();
    assert_eq!(links[0]["status"], "resolved");
    assert_eq!(links[0]["targets"][0]["path"], TARGET);
    assert_eq!(links[0]["targets"][0]["line"], 3);
    assert_eq!(links[1]["status"], "missing");
    assert_eq!(links[2]["status"], "missing_heading");
    assert!(links[2]["targets"].as_array().unwrap().is_empty());
    let (_, target) = response(
        app,
        &format!("/api/document?path={}&generation=1", encoded(TARGET)),
    )
    .await;
    assert_eq!(target["content"], TEXT);
    assert_eq!(target["navigation"]["outline"][1]["source"]["line"], 3);
    assert_eq!(
        target["navigation"]["backlinks"][0]["source"]["path"],
        SOURCE
    );
    assert_eq!(target["navigation"]["backlinks"][0]["source"]["line"], 2);
}

#[tokio::test]
async fn refresh_moves_search_and_navigation_together_and_rejects_old_generation() {
    let (vault, _assets, app) = fixture();
    fs::write(
        vault.path().join(TARGET),
        "# 改后说明\n\n\n## 阅读节奏\n新的字面内容\n",
    )
    .unwrap();
    let (_, before) = response(app.clone(), &search_uri("新的字面", "full_text", None, 1)).await;
    assert!(before["hits"].as_array().unwrap().is_empty());
    assert_eq!(
        response(app.clone(), "/api/workbench?refresh=true").await.0,
        StatusCode::OK
    );
    assert_eq!(
        response(app.clone(), &search_uri("连续阅读", "full_text", None, 1))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        response(
            app.clone(),
            &format!("/api/document?path={}&generation=1", encoded(SOURCE))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, after) = response(app.clone(), &search_uri("新的字面", "full_text", None, 2)).await;
    assert_eq!(after["hits"][0]["line"], 5);
    let (_, source) = response(
        app,
        &format!("/api/document?path={}&generation=2", encoded(SOURCE)),
    )
    .await;
    assert_eq!(source["navigation"]["outgoing"][0]["targets"][0]["line"], 4);
}

#[tokio::test]
async fn invalid_search_inputs_do_not_fall_back_to_a_broader_query() {
    let (_vault, _assets, app) = fixture();
    for uri in [
        search_uri("中文", "full_text", Some("项目/甲相似名"), 1),
        search_uri("x\ny", "full_text", None, 1),
        search_uri(&"字".repeat(257), "full_text", None, 1),
        search_uri("中文", "anything", None, 1),
    ] {
        assert_eq!(response(app.clone(), &uri).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        response(app, "/api/search?q=x&mode=full_text").await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn querying_keeps_source_bytes_and_directory_entries_unchanged() {
    let (vault, _assets, app) = fixture();
    let before = fs::read(vault.path().join(TARGET)).unwrap();
    let root_entries = fs::read_dir(vault.path()).unwrap().count();
    response(app.clone(), &search_uri("script", "full_text", None, 1)).await;
    response(
        app,
        &format!("/api/document?path={}&generation=1", encoded(TARGET)),
    )
    .await;
    assert_eq!(fs::read(vault.path().join(TARGET)).unwrap(), before);
    assert_eq!(fs::read_dir(vault.path()).unwrap().count(), root_entries);
    assert!(!vault.path().join(".workbench").exists());
}

#[tokio::test]
async fn new_search_surface_preserves_origin_and_readonly_guards() {
    let (_vault, _assets, app) = fixture();
    let uri = search_uri("中文", "full_text", None, 1);
    for (method, origin, expected) in [
        ("GET", "https://foreign.example", StatusCode::FORBIDDEN),
        (
            "POST",
            "http://127.0.0.1:4317",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
    ] {
        let result = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&uri)
                    .header("host", "127.0.0.1:4317")
                    .header("origin", origin)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(result.status(), expected);
    }
}
