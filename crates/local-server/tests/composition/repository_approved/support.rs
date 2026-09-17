use super::super::{Fixture, test_support};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use tower::ServiceExt;

pub const PROJECT: &str = "项目/虚构工坊";
const DESIGN: &str = "# 项目运作\n## 节点\n| ID | 名称 | 类型 | 目录 | 职责 |\n|---|---|---|---|---|\n| model | 人工模型 | 模块 | src | 合成职责 |\n## 关系\n| 起点 | 终点 | 类型 | 说明 |\n|---|---|---|---|\n## 任务路径\n| 流程ID | 流程名称 | 步骤 | 节点ID | 说明 |\n|---|---|---|---|---|\n";

pub fn configure(f: &Fixture, cargo: bool) -> PathBuf {
    let repository = f.root.join("repository");
    fs::create_dir(&repository).unwrap();
    fs::create_dir(repository.join("src")).unwrap();
    fs::write(
        repository.join("src/lib.rs"),
        "SOURCE_BODY_MUST_STAY_PRIVATE",
    )
    .unwrap();
    fs::write(
        repository.join("Cargo.toml"),
        "[package]\nname = \"synthetic\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let project = f.root.join("vault").join(PROJECT).join("01-框架");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("目录结构.md"),
        "## 代码仓库\n```text\nfixture/\n└── src/ # 人工职责\n```\n",
    )
    .unwrap();
    fs::write(project.join("项目运作.md"), DESIGN).unwrap();
    let other = f.root.join("vault/项目/其他/00-总览");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("INDEX.md"), "# 其他").unwrap();
    let mut value: Value = serde_json::from_slice(&fs::read(&f.config).unwrap()).unwrap();
    value["project_repository"] =
        json!({"version":1,"project":PROJECT,"directory":repository,"cargo_dependencies":cargo});
    fs::write(&f.config, serde_json::to_vec(&value).unwrap()).unwrap();
    repository
}

pub fn proxy(f: &Fixture) -> PathBuf {
    let path = f.root.join("proxy");
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path.join("http.sock")
}

pub fn request(f: &Fixture, method: &str, uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", format!("127.0.0.1:{}", f.port))
        .header("origin", "https://studiolo.example.com")
        .header(
            "cf-access-jwt-assertion",
            test_support::signed(&test_support::claims()),
        )
        .header("x-studiolo-request", "1")
        .header("sec-fetch-site", "same-origin")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-dest", "empty");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if method == "POST" {
        builder = builder.header("content-type", "application/json");
    }
    builder
        .body(if method == "POST" {
            Body::from("{}")
        } else {
            Body::empty()
        })
        .unwrap()
}

pub async fn cookie(f: &Fixture, app: &Router) -> String {
    issue(app, request(f, "POST", "/api/v1/sessions", None)).await
}

pub async fn issue(app: &Router, request: Request<Body>) -> String {
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), 201);
    response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

pub async fn json(app: &Router, request: Request<Body>, status: u16) -> Value {
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status().as_u16(), status);
    assert_eq!(response.headers()["cache-control"], "no-store");
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
        .unwrap_or(Value::Null)
}

pub async fn generation(f: &Fixture, app: &Router, cookie: &str) -> u64 {
    json(app, request(f, "GET", "/api/workbench", Some(cookie)), 200).await["generation"]
        .as_u64()
        .unwrap()
}

pub fn uri(project: &str, generation: u64) -> String {
    let project: String = project.bytes().map(|byte| format!("%{byte:02X}")).collect();
    format!("/api/project-operations?project={project}&generation={generation}")
}
