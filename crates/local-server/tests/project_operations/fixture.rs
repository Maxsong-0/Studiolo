use std::{fs, sync::Arc};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::Value;
use studiolo_local_server::{ProjectRepository, router, with_project_repository};
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;
use tower::ServiceExt;

pub const PROJECT: &str = "项目/虚构";
pub const DECLARATION: &str = "# 目录结构\n\n## 档案\n```text\n虚构/\n└── 机密档案/\n```\n\n## 代码仓库\n```text\nfixture/\n├── src/ # 中文职责 <script>不得执行</script>\n│   ├── model/ # 数据模型\n│   └── missing/ # 待核验\n└── docs/ # 说明\n```\n\n## 服务器\n```text\nserver/\n└── private/\n```\n";

pub struct Fixture {
    pub vault: TempDir,
    pub assets: TempDir,
    pub repository: TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let result = Self {
            vault: TempDir::new().unwrap(),
            assets: TempDir::new().unwrap(),
            repository: TempDir::new().unwrap(),
        };
        fs::create_dir_all(result.vault.path().join("项目/虚构/01-框架")).unwrap();
        fs::create_dir_all(result.vault.path().join("项目/其他/00-总览")).unwrap();
        fs::write(
            result.vault.path().join("项目/其他/00-总览/INDEX.md"),
            "# 其他",
        )
        .unwrap();
        result.write_declaration(DECLARATION);
        for dir in [
            "src/model",
            "src/extra",
            "src/model/internal",
            "docs",
            ".hidden/secret",
        ] {
            fs::create_dir_all(result.repository.path().join(dir)).unwrap();
        }
        fs::write(
            result.repository.path().join("src/source.rs"),
            "SOURCE_CONTENT_MUST_STAY_PRIVATE",
        )
        .unwrap();
        fs::write(
            result.assets.path().join("index.html"),
            "<!doctype html><title>fixture</title>",
        )
        .unwrap();
        result
    }

    pub fn write_declaration(&self, content: &str) {
        fs::write(
            self.vault.path().join("项目/虚构/01-框架/目录结构.md"),
            content,
        )
        .unwrap();
    }

    pub fn app(&self, bound: bool) -> Router {
        let app = router(
            ReadOnlyVault::open(self.vault.path()).unwrap(),
            ReadOnlyVault::open(self.assets.path()).unwrap(),
            "虚构图2".into(),
            4320,
        )
        .unwrap();
        if bound { self.bind(app) } else { app }
    }

    pub fn bind(&self, app: Router) -> Router {
        with_project_repository(
            app,
            Arc::new(ProjectRepository::open(PROJECT, self.repository.path()).unwrap()),
        )
    }
}

pub fn uri(project: &str, generation: u64) -> String {
    let encoded = project
        .bytes()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    format!("/api/project-operations?project={encoded}&generation={generation}")
}

pub async fn request(
    app: Router,
    uri: &str,
    method: &str,
    origin: &str,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .uri(uri)
        .method(method)
        .header("host", "127.0.0.1:4320")
        .header("origin", origin);
    if let Some(token) = token {
        builder = builder.header("cf-access-jwt-assertion", token);
    }
    let response = app
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub async fn get(app: Router, uri: &str) -> (StatusCode, Value) {
    request(app, uri, "GET", "http://127.0.0.1:4320", None).await
}

pub fn drift(value: &Value, path: &str) -> Option<String> {
    value["drift"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"] == path)
        .map(|row| row["status"].as_str().unwrap().to_owned())
}
