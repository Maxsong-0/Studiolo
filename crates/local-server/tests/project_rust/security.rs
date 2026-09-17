//! 真实文件系统的 Broker 拒绝边界；macOS 名称折叠与 Linux 区分大小写分别处理。
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

use super::test_support::{Fixture, empty, paths};

#[test]
fn symlinked_source_parent_and_replaced_root_never_follow_targets() {
    for target in ["src/api.rs", "src"] {
        let f = Fixture::new();
        let source = f.root.path().join(target);
        let outside = f.outside.path().join("retained");
        fs::rename(&source, &outside).unwrap();
        symlink(&outside, &source).unwrap();
        let result = f.observe();
        empty(&result, "unavailable");
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains(f.outside.path().to_str().unwrap())
        );
    }
    let f = Fixture::new();
    let repository = f.repository().with_rust_call_sites(paths()).unwrap();
    let outside = f.outside.path().join("old-root");
    fs::rename(f.root.path(), &outside).unwrap();
    symlink(&outside, f.root.path()).unwrap();
    empty(&repository.observe_rust_call_sites(), "unavailable");
    fs::remove_file(f.root.path()).unwrap();
}

#[test]
fn hard_links_non_files_and_case_collisions_cannot_form_source_evidence() {
    let f = Fixture::new();
    let linked = f.outside.path().join("linked.rs");
    fs::hard_link(f.root.path().join("src/api.rs"), &linked).unwrap();
    empty(&f.observe(), "unavailable");
    fs::remove_file(linked).unwrap();
    fs::remove_file(f.root.path().join("src/api.rs")).unwrap();
    fs::create_dir(f.root.path().join("src/api.rs")).unwrap();
    empty(&f.observe(), "unavailable");
    let f = Fixture::new();
    let collision = f.root.path().join("src/API.rs");
    // 折叠文件系统没有第二个独立条目；Linux 等大小写敏感卷必须拒绝碰撞。
    if !collision.exists() {
        fs::write(collision, "fn other() {}\n").unwrap();
        empty(&f.observe(), "unavailable");
    }
}

#[test]
fn missing_unreadable_and_first_round_non_utf8_are_atomic_failures() {
    let f = Fixture::new();
    let path = f.root.path().join("src/api.rs");
    fs::remove_file(&path).unwrap();
    empty(&f.observe(), "unavailable");
    fs::write(&path, [0xff, 0xfe]).unwrap();
    empty(&f.observe(), "invalid");
    if rustix::process::geteuid().as_raw() != 0 {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let result = f.observe();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        empty(&result, "unavailable");
    }
}
