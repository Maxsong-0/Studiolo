//! 独立 Cargo HTTP 夹具；临时目录内容全为虚构声明。

use std::{fs, sync::Arc};

use axum::Router;
use serde_json::Value;
use studiolo_local_server::{ProjectRepository, with_project_repository};

use crate::fixture::{DECLARATION, Fixture, PROJECT};

pub const ROOT: &str = "[workspace]\nmembers = [\"crates/api\", \"crates/model\"]\n[workspace.dependencies]\nmodel = { path = \"crates/model\" }\n";
pub const API: &str =
    "[package]\nname = \"api\"\nversion = \"0.1.0\"\n[dependencies]\nmodel.workspace = true\n";
pub const MODEL: &str = "[package]\nname = \"model\"\nversion = \"0.1.0\"\n[dependencies]\nserde = { git = \"https://SOURCE_SECRET@example.invalid/repo\" }\n";
pub const ARCHITECTURE: &str = "# 项目运作\n## 节点\n| ID | 名称 | 类型 | 目录 | 职责 |\n|---|---|---|---|---|\n| api | 人工接口设计 | 接口 | crates/api | 声明职责 |\n## 关系\n| 起点 | 终点 | 类型 | 说明 |\n|---|---|---|---|\n## 任务路径\n| 流程ID | 流程名称 | 步骤 | 节点ID | 说明 |\n|---|---|---|---|---|\n";

pub fn prepared() -> Fixture {
    // 与旧 helpers 共用形状，但本次全部目录明确建在 /tmp。
    let f = Fixture {
        vault: tempfile::TempDir::new_in("/tmp").unwrap(),
        assets: tempfile::TempDir::new_in("/tmp").unwrap(),
        repository: tempfile::TempDir::new_in("/tmp").unwrap(),
    };
    fs::create_dir_all(f.vault.path().join("项目/虚构/01-框架")).unwrap();
    fs::create_dir_all(f.vault.path().join("项目/其他/00-总览")).unwrap();
    fs::write(
        f.vault.path().join("项目/其他/00-总览/INDEX.md"),
        "# 其他项目",
    )
    .unwrap();
    f.write_declaration(DECLARATION);
    fs::write(
        f.vault.path().join("项目/虚构/01-框架/项目运作.md"),
        ARCHITECTURE,
    )
    .unwrap();
    for directory in [
        "src/model",
        "src/extra",
        "docs",
        "crates/api",
        "crates/model",
    ] {
        fs::create_dir_all(f.repository.path().join(directory)).unwrap();
    }
    fs::write(
        f.assets.path().join("index.html"),
        "<!doctype html><title>fixture</title>",
    )
    .unwrap();
    write(&f, "Cargo.toml", ROOT);
    write(&f, "crates/api/Cargo.toml", API);
    write(&f, "crates/model/Cargo.toml", MODEL);
    write(&f, "Cargo.lock", "LOCK_CONTENT_MUST_STAY_PRIVATE");
    write(&f, "build.rs", "SOURCE_CONTENT_MUST_STAY_PRIVATE");
    f
}

pub fn write(f: &Fixture, path: &str, content: &str) {
    fs::write(f.repository.path().join(path), content).unwrap();
}

pub fn bind(f: &Fixture, app: Router) -> Router {
    let repository = ProjectRepository::open(PROJECT, f.repository.path())
        .unwrap()
        .with_cargo_dependencies();
    with_project_repository(app, Arc::new(repository))
}

pub fn app(f: &Fixture) -> Router {
    bind(f, f.app(false))
}

pub fn empty(value: &Value, status: &str) {
    let cargo = &value["cargo_dependencies"];
    assert_eq!(cargo["status"], status);
    assert!(cargo["graph"].is_null());
    assert!(cargo["observed_at"].is_null());
}
