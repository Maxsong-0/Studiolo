//! 安全边界继承已有根/父 FD 实现；macOS 和 Linux 都必须通过链接与替换契约。
#![cfg(unix)]

use std::{
    fs,
    os::unix::{fs::symlink, net::UnixListener},
};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{BrokerError, ReadOnlyVault};
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
    // 根列表遇到链接项整次失败；以链接为选定路径或其父分量时在链接分量上拒绝。
    assert!(matches!(
        reader.list_directories(None, 8),
        Err(BrokerError::UnsafeNode {
            reason: "directory listing rejects links and special entries",
            ..
        })
    ));
    for prefix in ["linked", "linked/outside-only"] {
        let result = reader.list_directories(Some(&path(prefix)), 8);
        assert!(
            matches!(
                &result,
                Err(BrokerError::UnsafeNode {
                    path: node,
                    reason: "no-link rooted resolution failed",
                }) if *node == path("linked")
            ),
            "{prefix}: {:?}",
            result.err()
        );
    }
    assert!(matches!(
        ReadOnlyVault::open(&root.path().join("linked")),
        Err(BrokerError::Io {
            operation: "open read-only root",
            ..
        })
    ));
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
    assert!(matches!(
        reader.list_directories(None, 8),
        Err(BrokerError::UnsafeNode {
            reason: "directory listing rejects links and special entries",
            ..
        })
    ));
    drop(socket);
    fs::remove_file(root.path().join("socket")).unwrap();
    fs::write(root.path().join("file.rs"), b"fixture").unwrap();
    fs::hard_link(root.path().join("file.rs"), root.path().join("other.rs")).unwrap();
    // 两个名字谁先被枚举取决于目录顺序，只断言按硬链接拒绝。
    assert!(matches!(
        reader.list_directories(None, 8),
        Err(BrokerError::UnsafeNode {
            reason: "hard-linked or unlinked file",
            ..
        })
    ));
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
        assert!(matches!(
            reader.list_directories(None, 8),
            Err(BrokerError::Io {
                operation: "directory listing contains a non-UTF-8 name",
                ..
            })
        ));
        fs::remove_dir(root.path().join(non_utf8)).unwrap();
    }
    fs::create_dir(root.path().join("cafe\u{301}")).unwrap();
    assert!(matches!(
        reader.list_directories(None, 8),
        Err(BrokerError::Io {
            operation: "directory listing contains a noncanonical name",
            ..
        })
    ));
    fs::remove_dir(root.path().join("cafe\u{301}")).unwrap();
    fs::create_dir_all(root.path().join("Folder")).unwrap();
    fs::create_dir_all(root.path().join("folder")).unwrap();
    let mut names = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    let case_sensitive = names.len() == 2;
    // 分支自证：macOS 默认卷不允许大小写同名共存，Linux 等区分大小写的卷验证冲突；
    // 打印判据原始输入（实际目录项）与所选分支。
    eprintln!(
        "大小写目录·分支自证：目录项 = {names:?}，区分大小写 = {case_sensitive}（{}）",
        if case_sensitive {
            "两个拼写并存，整次列表失败"
        } else {
            "第二次创建落到同一目录，列表只含原拼写"
        }
    );
    if case_sensitive {
        // 两个拼写谁先被枚举取决于目录顺序，只断言按大小写冲突拒绝。
        assert!(matches!(
            reader.list_directories(None, 8),
            Err(BrokerError::UnsafeNode {
                reason: "case-colliding filesystem name",
                ..
            })
        ));
    } else {
        eprintln!("跳过：临时卷不区分大小写，无法构造并存的两个拼写；改验列表只返回原拼写");
        assert_eq!(names, ["Folder"]);
        assert_eq!(
            reader.list_directories(None, 8).unwrap(),
            vec![path("Folder")]
        );
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
