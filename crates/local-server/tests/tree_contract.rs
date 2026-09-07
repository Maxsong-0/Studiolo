//! 文件树的真实 Broker → 代次 → HTTP 契约；只使用临时虚构文件。

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

const ATTACHMENT: &str = "纯附件/资料 & 图表.pdf";
const BINARY: &[u8] = b"\xff\x00ATTACHMENT_CONTENT_MUST_NOT_ESCAPE";

fn fixture() -> (TempDir, TempDir, Router) {
    let vault = TempDir::new().unwrap();
    let assets = TempDir::new().unwrap();
    for directory in ["纯附件", "空目录", ".hidden"] {
        fs::create_dir(vault.path().join(directory)).unwrap();
    }
    fs::write(vault.path().join(ATTACHMENT), BINARY).unwrap();
    fs::write(vault.path().join("说明.MD"), "# 说明\n中文正文\n").unwrap();
    fs::write(vault.path().join(".hidden/私密.pdf"), "隐藏内容").unwrap();
    fs::write(
        assets.path().join("index.html"),
        "<!doctype html><title>文件树验收</title>",
    )
    .unwrap();
    let app = router(
        ReadOnlyVault::open(vault.path()).unwrap(),
        ReadOnlyVault::open(assets.path()).unwrap(),
        "虚构文件树".into(),
        4320,
    )
    .unwrap();
    (vault, assets, app)
}

fn encoded(value: &str) -> String {
    value.bytes().map(|byte| format!("%{byte:02X}")).collect()
}

fn tree_uri(path: &str, generation: u64, offset: usize) -> String {
    format!(
        "/api/tree?path={}&generation={generation}&offset={offset}",
        encoded(path)
    )
}

async fn request(app: Router, uri: &str, method: &str, origin: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .method(method)
                .header("host", "127.0.0.1:4320")
                .header("origin", origin)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get(app: Router, uri: &str) -> (StatusCode, Value) {
    request(app, uri, "GET", "http://127.0.0.1:4320").await
}

#[tokio::test]
async fn empty_and_attachment_only_directories_are_first_class_without_content() {
    let (_vault, _assets, app) = fixture();
    let (status, root) = get(app.clone(), &tree_uri("", 1, 0)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(root["generation"], 1);
    assert_eq!(root["total"], 3);
    assert_eq!(root["entries"][0]["kind"], "directory");
    assert_eq!(root["entries"][1]["kind"], "directory");
    assert_eq!(root["entries"][2]["kind"], "markdown");
    assert_eq!(
        root["entries"][2]["size_bytes"],
        "# 说明\n中文正文\n".len().to_string()
    );
    let (_, empty) = get(app.clone(), &tree_uri("空目录", 1, 0)).await;
    assert_eq!(empty["total"], 0);
    assert_eq!(empty["entries"], serde_json::json!([]));
    assert!(empty["next_offset"].is_null());
    let (_, files) = get(app.clone(), &tree_uri("纯附件", 1, 0)).await;
    assert_eq!(files["entries"][0]["path"], ATTACHMENT);
    assert_eq!(files["entries"][0]["kind"], "attachment");
    assert_eq!(files["entries"][0]["size_bytes"], BINARY.len().to_string());
    assert!(files["entries"][0]["child_count"].is_null());
    assert!(files["entries"][0].get("content").is_none());
    assert_eq!(
        get(app, &tree_uri(".hidden", 1, 0)).await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn attachment_bytes_have_no_document_search_or_download_route() {
    let (_vault, _assets, app) = fixture();
    for uri in [
        format!("/api/document?path={}&generation=1", encoded(ATTACHMENT)),
        format!("/api/attachment?path={}&generation=1", encoded(ATTACHMENT)),
        format!("/{}", encoded(ATTACHMENT)),
    ] {
        assert_eq!(get(app.clone(), &uri).await.0, StatusCode::NOT_FOUND);
    }
    let (_, result) = get(
        app,
        "/api/search?q=ATTACHMENT_CONTENT&mode=full_text&generation=1",
    )
    .await;
    assert_eq!(result["hits"], serde_json::json!([]));
}

#[tokio::test]
async fn metadata_and_markdown_advance_together_only_on_successful_refresh() {
    let (vault, _assets, app) = fixture();
    fs::write(vault.path().join(ATTACHMENT), b"new").unwrap();
    fs::write(vault.path().join("说明.MD"), "# 新正文").unwrap();
    let (_, before) = get(app.clone(), &tree_uri("纯附件", 1, 0)).await;
    assert_eq!(before["entries"][0]["size_bytes"], BINARY.len().to_string());
    let (_, changed) = get(app.clone(), "/api/workbench?refresh=true").await;
    assert_eq!(changed["generation"], 2);
    assert_eq!(
        get(app.clone(), &tree_uri("", 1, 0)).await.0,
        StatusCode::CONFLICT
    );
    let (_, after) = get(app.clone(), &tree_uri("纯附件", 2, 0)).await;
    assert_eq!(after["entries"][0]["size_bytes"], "3");
    let (_, note) = get(
        app.clone(),
        &format!("/api/document?path={}&generation=2", encoded("说明.MD")),
    )
    .await;
    assert_eq!(note["content"], "# 新正文");
    fs::write(vault.path().join(ATTACHMENT), b"unpublished attachment").unwrap();
    fs::write(vault.path().join("说明.MD"), "# 未发布正文").unwrap();
    fs::write(vault.path().join("错误.md"), b"\xff").unwrap();
    assert_eq!(
        get(app.clone(), "/api/workbench?refresh=true").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, preserved) = get(app.clone(), &tree_uri("纯附件", 2, 0)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preserved["generation"], 2);
    assert_eq!(preserved["entries"][0]["size_bytes"], "3");
    let (_, preserved_note) = get(
        app.clone(),
        &format!("/api/document?path={}&generation=2", encoded("说明.MD")),
    )
    .await;
    assert_eq!(preserved_note["content"], "# 新正文");
    let (_, search) = get(
        app,
        &format!(
            "/api/search?q={}&mode=full_text&generation=2",
            encoded("未发布正文")
        ),
    )
    .await;
    assert_eq!(search["hits"], serde_json::json!([]));
}

#[tokio::test]
async fn invalid_paths_offsets_generations_and_write_or_cross_origin_requests_fail() {
    let (_vault, _assets, app) = fixture();
    for path in ["../外部", "/absolute", "纯附件/../空目录", "说明.MD"] {
        assert_eq!(
            get(app.clone(), &tree_uri(path, 1, 0)).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        get(app.clone(), &tree_uri("缺失", 1, 0)).await.0,
        StatusCode::NOT_FOUND
    );
    for uri in [
        "/api/tree?path=",
        "/api/tree?generation=1",
        "/api/tree?path=&generation=1&offset=-1",
    ] {
        assert_eq!(get(app.clone(), uri).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        get(app.clone(), &tree_uri("", 1, 4)).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            app.clone(),
            &tree_uri("", 1, 0),
            "GET",
            "https://foreign.example"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(app, &tree_uri("", 1, 0), "POST", "http://127.0.0.1:4320")
            .await
            .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn directory_pages_are_bounded_and_complete() {
    let (vault, _assets, app) = fixture();
    for index in 0..205 {
        fs::write(vault.path().join(format!("纯附件/{index:03}.bin")), [0xff]).unwrap();
    }
    assert_eq!(
        get(app.clone(), "/api/workbench?refresh=true").await.0,
        StatusCode::OK
    );
    let (_, first) = get(app.clone(), &tree_uri("纯附件", 2, 0)).await;
    assert_eq!(first["total"], 206);
    assert_eq!(first["entries"].as_array().unwrap().len(), 200);
    assert_eq!(first["next_offset"], 200);
    let (_, last) = get(app, &tree_uri("纯附件", 2, 200)).await;
    assert_eq!(last["entries"].as_array().unwrap().len(), 6);
    assert!(last["next_offset"].is_null());
    assert_ne!(first["entries"][199]["path"], last["entries"][0]["path"]);
}

#[cfg(unix)]
#[tokio::test]
async fn unsafe_attachment_prevents_refresh_and_does_not_replace_old_generation() {
    let (vault, _assets, app) = fixture();
    std::os::unix::fs::symlink("资料 & 图表.pdf", vault.path().join("纯附件/链接.pdf")).unwrap();
    assert_eq!(
        get(app.clone(), "/api/workbench?refresh=true").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (_, page) = get(app, &tree_uri("纯附件", 1, 0)).await;
    assert_eq!(page["total"], 1);
}

fn fingerprint(root: &std::path::Path) -> Vec<(String, Option<Vec<u8>>)> {
    fn visit(
        root: &std::path::Path,
        directory: &std::path::Path,
        result: &mut Vec<(String, Option<Vec<u8>>)>,
    ) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let key = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                result.push((key, None));
                visit(root, &path, result);
            } else {
                result.push((key, Some(fs::read(path).unwrap())));
            }
        }
    }
    let mut result = Vec::new();
    visit(root, root, &mut result);
    result.sort();
    result
}

#[tokio::test]
async fn tree_reads_and_refresh_leave_all_source_bytes_and_directory_entries_unchanged() {
    let (vault, _assets, app) = fixture();
    let before = fingerprint(vault.path());
    for path in ["", "空目录", "纯附件"] {
        assert_eq!(
            get(app.clone(), &tree_uri(path, 1, 0)).await.0,
            StatusCode::OK
        );
    }
    assert_eq!(
        get(app, "/api/workbench?refresh=true").await.0,
        StatusCode::OK
    );
    assert_eq!(fingerprint(vault.path()), before);
}
