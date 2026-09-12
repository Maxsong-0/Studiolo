//! 仅供本模块使用的 /tmp 虚构源码夹具，所有 Span/语法树均由纯层内部处理。

use std::fs;

use crate::{ProjectRepository, project_rust::RustCallObservation};
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;

pub const API: &str =
    "mod hidden_module;\nfn serve() { helper(\"ARGUMENT_SECRET\"); include!(\"outside.rs\"); }\n";
pub const MODEL: &str = "fn helper() {}\n";

pub fn paths() -> Vec<String> {
    ["src/api.rs", "src/model.rs"].map(str::to_owned).to_vec()
}

pub struct Fixture {
    pub root: TempDir,
    pub outside: TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let f = Self {
            root: TempDir::new_in("/tmp").unwrap(),
            outside: TempDir::new_in("/tmp").unwrap(),
        };
        fs::create_dir(f.root.path().join("src")).unwrap();
        f.write("src/api.rs", API);
        f.write("src/model.rs", MODEL);
        f
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

    pub fn observe(&self) -> RustCallObservation {
        self.repository()
            .with_rust_call_sites(paths())
            .unwrap()
            .observe_rust_call_sites()
    }
}

pub fn empty(value: &RustCallObservation, status: &str) {
    assert_eq!(value.status, status);
    assert!(value.graph.is_none());
    assert!(value.observed_at.is_none());
    assert!(!value.diagnostics.is_empty());
}
