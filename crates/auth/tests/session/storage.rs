//! Auth 私有存储的目录、权限与对象身份契约；仅使用本次测试的临时文件。

#![cfg(unix)]

#[path = "storage_nodes.rs"]
mod nodes;

use super::{CreatePolicy, Storage};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
};

fn private_dir() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    mode(directory.path(), 0o700);
    directory
}

fn mode(path: &Path, value: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(value)).unwrap();
}

fn file(path: &Path, content: &[u8]) {
    fs::write(path, content).unwrap();
    mode(path, 0o600);
}

fn root(directory: &tempfile::TempDir) -> PathBuf {
    directory.path().canonicalize().unwrap()
}

fn fingerprint(path: &Path) -> (u64, u64, u32, u32, u64, u64) {
    let metadata = fs::symlink_metadata(path).unwrap();
    (
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.uid(),
        metadata.nlink(),
        metadata.len(),
    )
}

#[test]
fn new_database_is_private_and_created_is_reported_once() {
    let directory = private_dir();
    let root = root(&directory);
    let storage = Storage::prepare(&root, CreatePolicy::Allow).unwrap();
    assert!(storage.created);
    assert_eq!(storage.database, root.join("auth.sqlite3"));
    let metadata = fs::metadata(&storage.database).unwrap();
    assert!(metadata.is_file());
    assert_eq!(metadata.mode() & 0o7777, 0o600);
    assert_eq!(metadata.nlink(), 1);
    assert_eq!(metadata.uid(), fs::metadata(&root).unwrap().uid());
    assert_eq!(fs::metadata(&root).unwrap().mode() & 0o7777, 0o700);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    storage.check().unwrap();
    // 存储层只负责文件预检；实际 SQLite 格式由会话数据库层验证。
    file(&storage.database, b"nonempty storage fixture");
    let before = fingerprint(&storage.database);
    let reopened = Storage::prepare(&root, CreatePolicy::Allow).unwrap();
    assert!(!reopened.created);
    assert_eq!(fingerprint(&storage.database), before);
    assert_eq!(
        fs::read(&storage.database).unwrap(),
        b"nonempty storage fixture"
    );
}

#[test]
fn existing_empty_database_is_not_treated_as_a_new_database() {
    let directory = private_dir();
    let root = root(&directory);
    let database = root.join("auth.sqlite3");
    file(&database, b"");
    let before = fingerprint(&database);
    assert!(Storage::prepare(&root, CreatePolicy::Allow).is_err());
    assert_eq!(fingerprint(&database), before);
    assert!(fs::read(&database).unwrap().is_empty());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn directory_must_be_absolute_existing_and_not_a_regular_file() {
    let directory = private_dir();
    let root = root(&directory);
    assert!(Storage::prepare(Path::new("relative-auth-store"), CreatePolicy::Allow).is_err());
    let absent = root.join("missing");
    assert!(Storage::prepare(&absent, CreatePolicy::Allow).is_err());
    assert!(!absent.exists());
    let regular = root.join("regular");
    file(&regular, b"keep directory substitute");
    let before = fingerprint(&regular);
    assert!(Storage::prepare(&regular, CreatePolicy::Allow).is_err());
    assert_eq!(fingerprint(&regular), before);
    assert_eq!(fs::read(&regular).unwrap(), b"keep directory substitute");
}

#[test]
fn direct_and_ancestor_directory_symlinks_are_rejected() {
    let directory = private_dir();
    let root = root(&directory);
    let private = root.join("private");
    let child = private.join("child");
    fs::create_dir_all(&child).unwrap();
    mode(&private, 0o700);
    mode(&child, 0o700);
    let alias = root.join("alias");
    symlink(&private, &alias).unwrap();
    assert!(Storage::prepare(&alias, CreatePolicy::Allow).is_err());
    assert!(Storage::prepare(&alias.join("child"), CreatePolicy::Allow).is_err());
    assert!(!private.join("auth.sqlite3").exists());
    assert!(!child.join("auth.sqlite3").exists());
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn directory_permissions_must_be_exactly_private_without_repairing_them() {
    let directory = private_dir();
    let root = root(&directory);
    for permissions in [0o755, 0o770, 0o705, 0o1700] {
        mode(&root, permissions);
        let before = fingerprint(&root);
        assert!(Storage::prepare(&root, CreatePolicy::Allow).is_err());
        assert_eq!(fingerprint(&root), before);
        assert!(!root.join("auth.sqlite3").exists());
    }
    mode(&root, 0o700);
}

#[test]
fn database_replacement_and_disappearance_invalidate_the_original_handle() {
    let directory = private_dir();
    let root = root(&directory);
    let storage = Storage::prepare(&root, CreatePolicy::Allow).unwrap();
    file(&storage.database, b"original");
    let previous = root.join("previous.sqlite3");
    fs::rename(&storage.database, &previous).unwrap();
    assert!(storage.check().is_err());
    file(&storage.database, b"replacement");
    let before = fingerprint(&storage.database);
    assert!(storage.check().is_err());
    assert_eq!(fingerprint(&storage.database), before);
    assert_eq!(fs::read(&storage.database).unwrap(), b"replacement");
    assert_eq!(fs::read(previous).unwrap(), b"original");
}

#[test]
fn replacing_the_directory_cannot_reuse_a_previously_approved_path() {
    let directory = private_dir();
    let parent = root(&directory);
    let private = parent.join("private");
    fs::create_dir(&private).unwrap();
    mode(&private, 0o700);
    let storage = Storage::prepare(&private, CreatePolicy::Allow).unwrap();
    file(&storage.database, b"original");
    let previous = parent.join("previous");
    fs::rename(&private, &previous).unwrap();
    fs::create_dir(&private).unwrap();
    mode(&private, 0o700);
    file(&storage.database, b"replacement");
    assert!(storage.check().is_err());
    assert_eq!(fs::read(&storage.database).unwrap(), b"replacement");
    assert_eq!(
        fs::read(previous.join("auth.sqlite3")).unwrap(),
        b"original"
    );
}

#[test]
fn live_database_permissions_and_link_count_are_rechecked() {
    let directory = private_dir();
    let root = root(&directory);
    let storage = Storage::prepare(&root, CreatePolicy::Allow).unwrap();
    mode(&storage.database, 0o644);
    assert!(storage.check().is_err());
    mode(&storage.database, 0o600);
    storage.check().unwrap();
    let alias = root.join("hardlink");
    fs::hard_link(&storage.database, &alias).unwrap();
    assert!(storage.check().is_err());
    fs::remove_file(&alias).unwrap();
    storage.check().unwrap();
    mode(&root, 0o755);
    assert!(storage.check().is_err());
    mode(&root, 0o700);
    storage.check().unwrap();
}

#[test]
fn private_file_inspection_rejects_a_different_expected_owner() {
    let directory = private_dir();
    let path = root(&directory).join("auth.sqlite3");
    file(&path, b"owner fixture");
    let owner = fs::metadata(&path).unwrap().uid();
    let before = fingerprint(&path);
    // 不进行 chown 或提权；只验证私有检查函数的 owner 分支。
    assert!(super::inspect_file(&path, owner, false).unwrap().is_some());
    assert!(super::inspect_file(&path, owner.wrapping_add(1), false).is_err());
    assert_eq!(fingerprint(&path), before);
    assert_eq!(fs::read(path).unwrap(), b"owner fixture");
}
