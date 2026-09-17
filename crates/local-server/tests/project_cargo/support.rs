//! 本次创建的 /tmp Cargo 夹具；不运行 Cargo，不读取真实项目或用户配置。
#![allow(dead_code)]

use std::fs;

use crate::{ProjectRepository, project_cargo::CargoDependencyObservation};
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;

pub const ROOT: &str = "[workspace]\nmembers = [\"crates/api\", \"crates/model\"]\n[workspace.dependencies]\nmodel = { path = \"crates/model\" }\n";
pub const API: &str =
    "[package]\nname = \"api\"\nversion = \"0.1.0\"\n[dependencies]\nmodel.workspace = true\n";
pub const MODEL: &str = "[package]\nname = \"model\"\nversion = \"0.1.0\"\n";

pub struct Fixture {
    pub root: TempDir,
    pub outside: TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let result = Self {
            root: TempDir::new_in("/tmp").unwrap(),
            outside: TempDir::new_in("/tmp").unwrap(),
        };
        fs::create_dir_all(result.root.path().join("crates/api")).unwrap();
        fs::create_dir_all(result.root.path().join("crates/model")).unwrap();
        result.write("Cargo.toml", ROOT);
        result.write("crates/api/Cargo.toml", API);
        result.write("crates/model/Cargo.toml", MODEL);
        result
    }

    pub fn write(&self, path: &str, value: &str) {
        fs::write(self.root.path().join(path), value).unwrap();
    }

    pub fn reader(&self) -> ReadOnlyVault {
        ReadOnlyVault::open(self.root.path()).unwrap()
    }

    pub fn repository(&self) -> ProjectRepository {
        ProjectRepository::open("项目/虚构", self.root.path()).unwrap()
    }

    pub fn observe(&self) -> CargoDependencyObservation {
        self.repository()
            .with_cargo_dependencies()
            .observe_cargo_dependencies()
    }
}

pub fn empty(value: &CargoDependencyObservation, status: &str) {
    assert_eq!(value.status, status);
    assert!(value.graph.is_none());
    assert!(value.observed_at.is_none());
    assert!(!value.diagnostics.is_empty());
}
