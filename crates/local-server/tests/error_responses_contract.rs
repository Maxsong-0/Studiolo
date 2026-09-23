//! 错误响应契约：畸形查询串答复固定中文 JSON Problem，刷新失败不回显库内路径与底层
//! 英文错误，扫描诊断有界并注明总数。所有源数据均为临时虚构夹具。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::CONTENT_TYPE},
};
use serde_json::Value;
use std::{fs, os::unix::fs::symlink, path::Path};
use studiolo_local_server::{LocalError, router};
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;
use tower::ServiceExt;

const NOTE: &str =
    "---\nlifecycle: 活跃\n---\n# 测试项目\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | M0 |\n";
const INVALID_QUERY: &str = "查询参数不合法。";
const REFRESH_FAILED: &str = "刷新派生失败，请查看服务端日志。";

/// 被拒绝的字段名与解析器英文细节；任何一个出现在正文里都算回显。
const LEAKS: [&str; 12] = [
    "generation",
    "path",
    "mode",
    "offset",
    "refresh",
    "project",
    "root",
    "missing",
    "unknown",
    "field",
    "deserialize",
    "Failed",
];

fn vault() -> TempDir {
    let vault = TempDir::new().unwrap();
    fs::create_dir_all(vault.path().join("项目/测试项目/00-总览")).unwrap();
    fs::write(vault.path().join("项目/测试项目/00-总览/INDEX.md"), NOTE).unwrap();
    vault
}

fn app(vault: &Path) -> (TempDir, Result<Router, LocalError>) {
    let assets = TempDir::new().unwrap();
    fs::write(assets.path().join("index.html"), "<!doctype html>").unwrap();
    let app = router(
        ReadOnlyVault::open(vault).unwrap(),
        ReadOnlyVault::open(assets.path()).unwrap(),
        "测试副本".into(),
        4317,
    );
    (assets, app)
}

/// 每条诊断都是一条跳过的符号链接；扫描按字典序给出诊断。
fn links(vault: &Path, names: impl Iterator<Item = String>) {
    for name in names {
        symlink("00-总览/INDEX.md", vault.join("项目/测试项目").join(name)).unwrap();
    }
}

async fn reply(app: &Router, uri: &str) -> (StatusCode, String, String) {
    let request = Request::builder()
        .uri(uri)
        .header("host", "127.0.0.1:4317")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        content_type,
        String::from_utf8(body.to_vec()).unwrap(),
    )
}

#[tokio::test]
async fn malformed_query_strings_answer_fixed_chinese_json_without_field_names() {
    let vault = vault();
    let (_assets, app) = app(vault.path());
    let app = app.unwrap();
    for (uri, code) in [
        ("/api/search?q=x&mode=full_text", "invalid_search_query"),
        (
            "/api/search?q=x&mode=full_text&generation=abc",
            "invalid_search_query",
        ),
        (
            "/api/search?q=x&mode=anything&generation=1",
            "invalid_search_query",
        ),
        ("/api/document?generation=1", "invalid_document_query"),
        (
            "/api/document?path=a.md&generation=-1",
            "invalid_document_query",
        ),
        ("/api/tree?path=", "invalid_tree_query"),
        (
            "/api/tree?path=&generation=1&offset=-1",
            "invalid_tree_query",
        ),
        ("/api/workbench?refresh=maybe", "invalid_workbench_query"),
        (
            "/api/project-operations?project=a",
            "invalid_operations_query",
        ),
        (
            "/api/project-operations?project=a&generation=1&root=/tmp",
            "invalid_operations_query",
        ),
    ] {
        let (status, content_type, body) = reply(&app, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(content_type, "application/json", "{uri}");
        let value: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["code"], code, "{uri}");
        assert_eq!(value["error"], INVALID_QUERY, "{uri}");
        for leaked in LEAKS {
            assert!(!body.contains(leaked), "{uri} 回显了 {leaked}：{body}");
        }
    }
    // 合法查询串照常答复，拒绝改造不影响正常路径。
    let (status, _, body) = reply(&app, "/api/tree?path=&generation=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_failed_refresh_answers_a_fixed_sentence_without_vault_paths() {
    let vault = vault();
    let (_assets, app) = app(vault.path());
    let app = app.unwrap();
    let secret = vault.path().join("项目/测试项目/私密草稿-draft.md");
    fs::write(&secret, [0xff, 0xfe]).unwrap();
    let (status, content_type, body) = reply(&app, "/api/workbench?refresh=true").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(content_type, "application/json");
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["error"], REFRESH_FAILED);
    for leaked in ["私密草稿", "draft", ".md", "项目/", "UTF-8"] {
        assert!(!body.contains(leaked), "刷新失败回显了 {leaked}：{body}");
    }

    fs::remove_file(&secret).unwrap();
    links(
        vault.path(),
        (0..25).map(|index| format!("link-{index:02}.md")),
    );
    let (status, _, body) = reply(&app, "/api/workbench?refresh=true").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        value["error"],
        format!("{REFRESH_FAILED}副本扫描共有 25 条诊断。")
    );
    for leaked in ["link", "symbolic", ".md", "项目/"] {
        assert!(!body.contains(leaked), "刷新失败回显了 {leaked}：{body}");
    }
    // 失败不发布半成品代次：首屏仍停在初扫那一代。
    let (status, _, body) = reply(&app, "/api/workbench").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["generation"],
        1
    );
}

#[tokio::test]
async fn scan_diagnostics_list_at_most_twenty_bounded_entries_with_the_total() {
    let vault = vault();
    // 一条超长诊断排在最前，用来核对逐条截断；其余 24 条普通诊断。
    let long = format!("{}.md", "a".repeat(240));
    links(
        vault.path(),
        std::iter::once(long).chain((0..24).map(|index| format!("link-{index:02}.md"))),
    );
    let (_assets, app) = app(vault.path());
    let Err(error) = app else {
        panic!("不完整扫描不能装配出路由");
    };
    let LocalError::Incomplete { total, listed } = &error else {
        panic!("应为扫描不完整，实际为 {error}");
    };
    assert_eq!(*total, 25);
    let parts = listed.split('；').collect::<Vec<_>>();
    assert_eq!(parts.len(), 21, "{listed}");
    assert_eq!(parts[20], "另有 5 条诊断未列出");
    assert!(parts[0].starts_with(&format!("项目/测试项目/{}", "a".repeat(40))));
    assert_eq!(parts[0].chars().count(), 201, "逐条截断到 200 字符加省略号");
    assert!(parts[0].ends_with('…'));
    for (index, part) in parts[1..20].iter().enumerate() {
        assert_eq!(
            *part,
            format!("项目/测试项目/link-{index:02}.md: symbolic link skipped")
        );
    }
    assert!(!listed.contains("link-19.md"), "第 21 条起只计数不列出");
    let shown = error.to_string();
    assert!(shown.contains("共 25 条诊断"), "{shown}");
    assert!(shown.contains("另有 5 条诊断未列出"), "{shown}");
}
