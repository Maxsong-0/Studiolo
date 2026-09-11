//! 仓库绑定使用的直属目录枚举契约；全部夹具均在本次创建的 /tmp 目录内。
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    time::{Duration, UNIX_EPOCH},
};
use studiolo_core::VaultPath;
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;

fn temporary() -> TempDir {
    TempDir::new_in("/tmp").unwrap()
}

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

#[test]
fn empty_root_and_empty_child_return_complete_empty_results() {
    let root = temporary();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    assert_eq!(reader.list_directories(None, 1).unwrap(), Vec::new());
    fs::create_dir(root.path().join("空目录")).unwrap();
    assert_eq!(
        reader.list_directories(None, 1).unwrap(),
        vec![path("空目录")]
    );
    assert!(
        reader
            .list_directories(Some(&path("空目录")), 1)
            .unwrap()
            .is_empty()
    );
    assert!(!root.path().join(".workbench").exists());
}

#[test]
fn directories_are_direct_sorted_root_relative_and_files_are_metadata_only() {
    let root = temporary();
    for name in [
        "资料/空目录",
        "资料/代码/更深",
        "另一个",
        ".git",
        "资料/.隐藏",
    ] {
        fs::create_dir_all(root.path().join(name)).unwrap();
    }
    let sealed = root.path().join("资料/秘密.md");
    fs::write(&sealed, [0xff, 0xfe, 0]).unwrap();
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000)).unwrap();
    let source_path = root.path().join("资料/源代码.rs");
    let source = fs::File::create(&source_path).unwrap();
    source.set_len(1024 * 1024 * 1024 * 1024).unwrap();
    source
        .set_times(fs::FileTimes::new().set_accessed(UNIX_EPOCH + Duration::from_secs(1)))
        .unwrap();
    let before = fs::metadata(&source_path).unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    let root_result = reader.list_directories(None, 3).unwrap();
    let result = reader.list_directories(Some(&path("资料")), 5);
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(root_result, vec![path("另一个"), path("资料")]);
    assert_eq!(
        result.unwrap(),
        vec![path("资料/代码"), path("资料/空目录")]
    );
    let after = fs::metadata(&source_path).unwrap();
    assert_eq!(before.accessed().unwrap(), after.accessed().unwrap());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_eq!(before.len(), after.len());
    assert_eq!(fs::read(&sealed).unwrap(), [0xff, 0xfe, 0]);
    assert!(!root.path().join(".workbench").exists());
    assert!(!root.path().join("资料/.workbench").exists());
}

#[test]
fn files_and_hidden_entries_use_the_budget_and_overflow_returns_no_partial_result() {
    let root = temporary();
    fs::create_dir(root.path().join("folder")).unwrap();
    fs::write(root.path().join("source.rs"), b"fixture").unwrap();
    fs::write(root.path().join(".hidden"), b"fixture").unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    assert!(reader.list_directories(None, 2).is_err());
    assert_eq!(
        reader.list_directories(None, 3).unwrap(),
        vec![path("folder")]
    );
    for budget in [0, 8193, usize::MAX] {
        assert!(reader.list_directories(None, budget).is_err());
    }
    assert_eq!(
        reader.list_directories(None, 8192).unwrap(),
        vec![path("folder")]
    );
}

#[test]
fn caller_paths_reject_traversal_hidden_components_files_and_missing_directories() {
    let root = temporary();
    fs::create_dir_all(root.path().join(".git/objects")).unwrap();
    fs::write(root.path().join("source.rs"), b"fixture").unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    for value in [
        "",
        ".",
        "..",
        "../outside",
        "/tmp",
        "a/../b",
        "a//b",
        "a\\b",
    ] {
        assert!(VaultPath::parse(value).is_err(), "{value}");
    }
    for value in [".git", ".git/objects", "source.rs", "missing"] {
        assert!(
            reader.list_directories(Some(&path(value)), 8).is_err(),
            "{value}"
        );
    }
}

#[test]
fn a_budget_failure_on_a_directory_of_only_files_is_not_an_empty_success() {
    let root = temporary();
    for index in 0..9 {
        fs::write(root.path().join(format!("file-{index}.rs")), []).unwrap();
    }
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    assert!(reader.list_directories(None, 8).is_err());
    assert!(reader.list_directories(None, 9).unwrap().is_empty());
}
