//! 不完整诊断不能绕过聚合逻辑路径内存预算。
#![cfg(unix)]

use std::fs;
use studiolo_vault_broker::{ReadOnlyVault, ScanLimits};
use tempfile::TempDir;

#[test]
fn noncanonical_diagnostics_cannot_bypass_the_total_path_budget() {
    let root = TempDir::new().unwrap();
    let component = "p".repeat(200);
    let directory = root
        .path()
        .join(&component)
        .join(&component)
        .join(&component);
    fs::create_dir_all(&directory).unwrap();
    // 总条目不足 50k，但每项完整路径约 804 字节；无内容分配，适用于 macOS 路径上限。
    for index in 0..21_000 {
        let name = format!("{index:05}{}\u{1}", "x".repeat(195));
        fs::write(directory.join(name), []).unwrap();
    }
    let result = ReadOnlyVault::open(root.path())
        .unwrap()
        .scan(ScanLimits::default());
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains("logical path bytes limit exceeded"),
        "{error}"
    );
    assert!(!error.contains(&root.path().display().to_string()));
    assert!(!root.path().join(".workbench").exists());
}
