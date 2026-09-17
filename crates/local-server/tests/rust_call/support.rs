//! Rust HTTP 使用独立 /tmp 源码，借用既有目录/Cargo 夹具以验证证据互相独立。

use std::sync::Arc;

use axum::Router;
use serde_json::Value;
use studiolo_local_server::{ProjectRepository, with_project_repository};

use crate::{
    cargo,
    fixture::{Fixture, PROJECT},
};

pub const API: &str = "mod ignored;\npub fn serve() {\n    helper(\"ARGUMENT_SECRET\");\n    value.send();\n    (callback)();\n    include!(\"../excluded.rs\");\n}\n";

pub fn paths() -> Vec<String> {
    ["src/api.rs", "src/model.rs"].map(str::to_owned).to_vec()
}

pub fn prepared() -> Fixture {
    let f = cargo::prepared();
    cargo::write(&f, "src/api.rs", API);
    cargo::write(&f, "src/model.rs", "pub fn helper() {}\n");
    cargo::write(&f, "src/ignored.rs", "IGNORED_MODULE_SECRET");
    cargo::write(&f, "excluded.rs", "INCLUDED_SOURCE_SECRET");
    f
}

pub fn bind(f: &Fixture, app: Router, cargo_enabled: bool) -> Router {
    let repository = ProjectRepository::open(PROJECT, f.repository.path())
        .unwrap()
        .with_rust_call_sites(paths())
        .unwrap();
    let repository = if cargo_enabled {
        repository.with_cargo_dependencies()
    } else {
        repository
    };
    with_project_repository(app, Arc::new(repository))
}

pub fn app(f: &Fixture) -> Router {
    bind(f, f.app(false), true)
}

pub fn empty(value: &Value, status: &str) {
    let observation = &value["rust_call_sites"];
    assert_eq!(observation["status"], status);
    assert!(observation["graph"].is_null());
    assert!(observation["observed_at"].is_null());
    assert!(!observation["diagnostics"].as_array().unwrap().is_empty());
}
