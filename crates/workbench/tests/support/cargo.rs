//! Cargo 纯契约共享的显式输入，无任何文件系统读取。
#![allow(dead_code)]

use studiolo_workbench::{CargoDependencyError, CargoDependencyErrorKind, CargoManifest};

pub fn manifest(path: &str, content: &str) -> CargoManifest {
    CargoManifest {
        path: path.to_owned(),
        content: content.to_owned(),
    }
}

pub fn package(name: &str, body: &str) -> String {
    format!("[package]\nname = '{name}'\n{body}\n")
}

pub fn pair(body: &str) -> Vec<CargoManifest> {
    vec![
        manifest(
            "Cargo.toml",
            "[workspace]\nmembers = ['app', 'core']\n[workspace.dependencies]\ncore = {path='core'}\nexternal = '1'\n",
        ),
        manifest("app/Cargo.toml", &package("app", body)),
        manifest("core/Cargo.toml", &package("core", "")),
    ]
}

pub fn error<T: std::fmt::Debug>(
    result: Result<T, CargoDependencyError>,
    kind: CargoDependencyErrorKind,
) {
    let error = result.expect_err("该声明必须整图拒绝");
    assert_eq!(error.kind, kind);
    assert!(!error.message.is_empty());
    assert!(!error.message.contains("PRIVATE_VALUE"));
}
