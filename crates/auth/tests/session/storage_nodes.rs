//! 数据库与三个 SQLite 边文件的特殊节点负测；拒绝时保留原文件和链接目标。

use super::{CreatePolicy, Storage, file, fingerprint, mode, private_dir, root};
use std::{
    fs,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, symlink},
        net::UnixListener,
    },
};

const SUFFIXES: [&str; 4] = ["", "-wal", "-shm", "-journal"];

#[test]
fn regular_private_sidecars_with_the_same_owner_are_accepted() {
    let directory = private_dir();
    let root = root(&directory);
    for suffix in &SUFFIXES[1..] {
        file(
            &root.join(format!("auth.sqlite3{suffix}")),
            b"sidecar fixture",
        );
    }
    let storage = Storage::prepare(&root, CreatePolicy::Allow).unwrap();
    assert!(storage.created);
    storage.check().unwrap();
    for suffix in &SUFFIXES[1..] {
        let path = root.join(format!("auth.sqlite3{suffix}"));
        let metadata = fs::metadata(&path).unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(metadata.nlink(), 1);
        assert_eq!(metadata.uid(), fs::metadata(&root).unwrap().uid());
        assert_eq!(fs::read(path).unwrap(), b"sidecar fixture");
    }
}

#[test]
fn database_and_sidecar_permissions_are_rejected_without_chmod_or_truncation() {
    for suffix in SUFFIXES {
        for permissions in [0o644, 0o660, 0o400, 0o1600] {
            let directory = private_dir();
            let root = root(&directory);
            let target = root.join(format!("auth.sqlite3{suffix}"));
            file(&target, b"keep existing bytes");
            mode(&target, permissions);
            let before = fingerprint(&target);
            assert!(Storage::prepare(&root, CreatePolicy::Allow).is_err());
            assert_eq!(fingerprint(&target), before);
            assert_eq!(fs::read(&target).unwrap(), b"keep existing bytes");
            if !suffix.is_empty() {
                assert!(!root.join("auth.sqlite3").exists());
            }
        }
    }
}

#[test]
fn live_and_dangling_symlinks_never_follow_or_modify_the_target() {
    for suffix in SUFFIXES {
        for existing in [true, false] {
            let directory = private_dir();
            let root = root(&directory);
            let outside = root.join("outside");
            if existing {
                file(&outside, b"outside must survive");
            }
            let target = root.join(format!("auth.sqlite3{suffix}"));
            symlink(&outside, &target).unwrap();
            let before = fingerprint(&target);
            assert!(Storage::prepare(&root, CreatePolicy::Allow).is_err());
            assert_eq!(fingerprint(&target), before);
            assert!(
                fs::symlink_metadata(&target)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            if existing {
                assert_eq!(fs::read(&outside).unwrap(), b"outside must survive");
            } else {
                assert!(!outside.exists());
            }
        }
    }
}

#[test]
fn multiple_hardlinks_are_rejected_for_every_managed_filename() {
    for suffix in SUFFIXES {
        let directory = private_dir();
        let root = root(&directory);
        let outside = root.join("original");
        file(&outside, b"shared file must survive");
        let target = root.join(format!("auth.sqlite3{suffix}"));
        fs::hard_link(&outside, &target).unwrap();
        let before = fingerprint(&target);
        assert_eq!(fs::metadata(&target).unwrap().nlink(), 2);
        assert!(Storage::prepare(&root, CreatePolicy::Allow).is_err());
        assert_eq!(fingerprint(&target), before);
        assert_eq!(fs::read(&outside).unwrap(), b"shared file must survive");
    }
}

#[test]
fn sockets_and_directories_are_not_opened_as_database_or_sidecars() {
    for suffix in SUFFIXES {
        let directory = private_dir();
        let root = root(&directory);
        let target = root.join(format!("auth.sqlite3{suffix}"));
        let _socket = UnixListener::bind(&target).unwrap();
        mode(&target, 0o600);
        let before = fingerprint(&target);
        assert!(Storage::prepare(&root, CreatePolicy::Allow).is_err());
        assert_eq!(fingerprint(&target), before);
        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_socket()
        );
    }
    for suffix in SUFFIXES {
        let directory = private_dir();
        let root = root(&directory);
        let target = root.join(format!("auth.sqlite3{suffix}"));
        fs::create_dir(&target).unwrap();
        mode(&target, 0o700);
        assert!(Storage::prepare(&root, CreatePolicy::Allow).is_err());
        assert!(target.is_dir());
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    }
}

#[test]
fn unsafe_sidecars_added_after_prepare_invalidate_each_live_check() {
    for suffix in &SUFFIXES[1..] {
        let directory = private_dir();
        let root = root(&directory);
        let storage = Storage::prepare(&root, CreatePolicy::Allow).unwrap();
        let target = root.join(format!("auth.sqlite3{suffix}"));
        file(&target, b"sidecar must survive");
        storage.check().unwrap();
        mode(&target, 0o644);
        assert!(storage.check().is_err());
        mode(&target, 0o600);
        storage.check().unwrap();
        let original = root.join("original-sidecar");
        fs::rename(&target, &original).unwrap();
        symlink(&original, &target).unwrap();
        assert!(storage.check().is_err());
        assert_eq!(fs::read(original).unwrap(), b"sidecar must survive");
    }
}
