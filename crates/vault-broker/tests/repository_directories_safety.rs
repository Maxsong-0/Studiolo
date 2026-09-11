//! 安全边界继承已有根/父 FD 实现；macOS 和 Linux 都必须通过链接与替换契约。
#![cfg(unix)]

use std::{
    fs,
    os::unix::{fs::symlink, net::UnixListener},
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
fn links_in_the_selected_path_and_visible_listing_fail_without_partial_results() {
    let root = temporary();
    let outside = temporary();
    fs::create_dir(outside.path().join("outside-only")).unwrap();
    fs::create_dir(root.path().join("safe")).unwrap();
    symlink(outside.path(), root.path().join("linked")).unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    for prefix in [
        None,
        Some(path("linked")),
        Some(path("linked/outside-only")),
    ] {
        assert!(reader.list_directories(prefix.as_ref(), 8).is_err());
    }
    assert!(ReadOnlyVault::open(&root.path().join("linked")).is_err());
    fs::remove_file(root.path().join("linked")).unwrap();
    symlink(outside.path(), root.path().join(".hidden-link")).unwrap();
    assert_eq!(
        reader.list_directories(None, 2).unwrap(),
        vec![path("safe")]
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
}

#[test]
fn pinned_root_does_not_switch_to_a_replacement_directory_or_symlink() {
    let parent = temporary();
    let selected = parent.path().join("selected");
    fs::create_dir_all(selected.join("original/child")).unwrap();
    let reader = ReadOnlyVault::open(&selected).unwrap();
    let child_reader = reader.open_directory(&path("original")).unwrap();
    fs::rename(&selected, parent.path().join("retained")).unwrap();
    fs::create_dir_all(selected.join("replacement")).unwrap();
    assert_eq!(
        reader.list_directories(None, 8).unwrap(),
        vec![path("original")]
    );
    assert_eq!(
        child_reader.list_directories(None, 8).unwrap(),
        vec![path("child")]
    );
    fs::rename(&selected, parent.path().join("replacement-root")).unwrap();
    symlink(parent.path().join("replacement-root"), &selected).unwrap();
    assert_eq!(
        reader.list_directories(None, 8).unwrap(),
        vec![path("original")]
    );
    assert_eq!(
        reader.list_directories(Some(&path("original")), 8).unwrap(),
        vec![path("original/child")]
    );
}

#[test]
fn special_types_and_hard_links_fail_the_complete_listing() {
    let root = temporary();
    fs::create_dir(root.path().join("safe")).unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    let socket = UnixListener::bind(root.path().join("socket")).unwrap();
    assert!(reader.list_directories(None, 8).is_err());
    drop(socket);
    fs::remove_file(root.path().join("socket")).unwrap();
    fs::write(root.path().join("file.rs"), b"fixture").unwrap();
    fs::hard_link(root.path().join("file.rs"), root.path().join("other.rs")).unwrap();
    assert!(reader.list_directories(None, 8).is_err());
}

#[test]
fn noncanonical_names_and_case_collisions_fail_closed() {
    let root = temporary();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    // Linux 目录名允许任意非 NUL 字节；macOS 卷可能在创建时就拒绝非法 UTF-8。
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStringExt;
        let non_utf8 = std::ffi::OsString::from_vec(vec![0xff]);
        fs::create_dir(root.path().join(&non_utf8)).unwrap();
        assert!(reader.list_directories(None, 8).is_err());
        fs::remove_dir(root.path().join(non_utf8)).unwrap();
    }
    fs::create_dir(root.path().join("cafe\u{301}")).unwrap();
    assert!(reader.list_directories(None, 8).is_err());
    fs::remove_dir(root.path().join("cafe\u{301}")).unwrap();
    fs::create_dir_all(root.path().join("Folder")).unwrap();
    fs::create_dir_all(root.path().join("folder")).unwrap();
    // macOS 默认卷可能不允许大小写同名共存；Linux 等区分大小写的卷验证冲突。
    if fs::read_dir(root.path()).unwrap().count() == 2 {
        assert!(reader.list_directories(None, 8).is_err());
    }
}

#[test]
fn directory_and_link_replacement_never_returns_outside_names() {
    use std::{
        sync::{
            Arc, Barrier,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };
    let root = temporary();
    let outside = temporary();
    let moving = root.path().join("moving");
    let parked = root.path().join(".parked");
    fs::create_dir_all(moving.join("safe-child")).unwrap();
    fs::create_dir(outside.path().join("outside-only")).unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(2));
    let writer_stop = Arc::clone(&stop);
    let writer_barrier = Arc::clone(&barrier);
    let outside_path = outside.path().to_owned();
    let writer = thread::spawn(move || {
        writer_barrier.wait();
        while !writer_stop.load(Ordering::Acquire) {
            fs::rename(&moving, &parked).unwrap();
            symlink(&outside_path, &moving).unwrap();
            fs::remove_file(&moving).unwrap();
            fs::rename(&parked, &moving).unwrap();
        }
    });
    barrier.wait();
    let mut escaped = false;
    for _ in 0..128 {
        if let Ok(listing) = reader.list_directories(Some(&path("moving")), 8) {
            escaped |= listing != vec![path("moving/safe-child")];
        }
    }
    stop.store(true, Ordering::Release);
    writer.join().unwrap();
    assert!(!escaped, "成功返回不能包含链接目标目录或不完整结果");
    assert_eq!(
        reader.list_directories(Some(&path("moving")), 8).unwrap(),
        vec![path("moving/safe-child")]
    );
}
