//! `/api/workbench` 首屏契约：只给计数不给逐篇清单，并发刷新一律得到答复。
//!
//! 所有源数据均为临时虚构夹具，不读取任何真实笔记。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::Value;
use std::fs;
use studiolo_local_server::router;
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;
use tower::ServiceExt;

const NOTE: &str = "---\nlifecycle: 活跃\n---\n# 测试项目\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | M0 |\n\n## 用户侧待办\n\n- 待验收：确认只读工作台\n";

fn fixture() -> (TempDir, Router) {
    let vault = TempDir::new().unwrap();
    let assets = TempDir::new().unwrap();
    fs::create_dir_all(vault.path().join("项目/测试项目/00-总览")).unwrap();
    fs::write(vault.path().join("项目/测试项目/00-总览/INDEX.md"), NOTE).unwrap();
    fs::write(vault.path().join("项目/测试项目/笔记.md"), "# 另一篇\n").unwrap();
    fs::create_dir(assets.path().join("assets")).unwrap();
    fs::write(
        assets.path().join("index.html"),
        "<!doctype html><title>测试</title>",
    )
    .unwrap();
    let app = router(
        ReadOnlyVault::open(vault.path()).unwrap(),
        ReadOnlyVault::open(assets.path()).unwrap(),
        "测试副本".into(),
        4317,
    )
    .unwrap();
    (vault, app)
}

fn request(uri: &str) -> axum::http::request::Builder {
    Request::builder().uri(uri).header("host", "127.0.0.1:4317")
}

async fn body(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn the_workbench_response_reports_document_counts_instead_of_a_per_document_list() {
    let (_vault, app) = fixture();
    let response = app
        .oneshot(request("/api/workbench").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let snapshot = body(response).await;

    let mut fields: Vec<_> = snapshot
        .as_object()
        .expect("首屏响应是一个 JSON 对象")
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        [
            "counts",
            "diagnostics",
            "generation",
            "inbox",
            "projects",
            "source_label"
        ],
        "首屏不再下发逐篇文档清单，只多一个 counts"
    );
    assert_eq!(snapshot["counts"]["documents"]["value"], 2);
    assert!(
        snapshot["counts"]["documents"]["unavailable"]
            .as_array()
            .expect("计数带显式不可得说明")
            .is_empty()
    );
    assert_eq!(snapshot["counts"]["projects"]["value"], 1);
    assert_eq!(snapshot["counts"]["inbox"]["value"], 1);
    // 夹具项目没有可解析的进行中任务数，汇总必须显式不可得而不是退化成 0。
    assert!(snapshot["counts"]["active_tasks"]["value"].is_null());
    assert!(
        !snapshot["counts"]["active_tasks"]["unavailable"]
            .as_array()
            .expect("不可得计数逐条说明来源")
            .is_empty()
    );
    assert_eq!(snapshot["projects"][0]["name"], "测试项目");
}

#[tokio::test]
async fn concurrent_refresh_requests_are_all_answered_instead_of_being_squeezed_out_of_work_slots()
{
    let (_vault, app) = fixture();
    let requests: Vec<_> = (0..9)
        .map(|_| {
            let app = app.clone();
            tokio::spawn(async move {
                app.oneshot(
                    request("/api/workbench?refresh=true")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
            })
        })
        .collect();
    for request in requests {
        let response = request.await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "并发刷新合流之后不得在投影这一步被工作槽挤掉"
        );
        assert!(body(response).await["counts"]["documents"]["value"] == 2);
    }
}
