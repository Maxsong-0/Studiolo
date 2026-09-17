use super::*;
use std::os::unix::{ffi::OsStringExt, fs::symlink};

#[test]
fn unapproved_required_paths_fail_before_metadata_or_body_io() {
    let f = Fixture::new();
    let mut broker = f.broker();
    fs::create_dir(f.root.join("notes")).unwrap();
    for name in [
        "outside.md",
        "notes/TASKS.md",
        "notes/generated.md",
        "notes/private/a",
    ] {
        reset();
        assert!(
            broker
                .capture_sync_observation(&selection(), &[path(name)])
                .is_err()
        );
        assert_eq!(count(0), 0);
        assert_eq!(count(1), 0);
    }
}

#[test]
fn selected_missing_anchor_is_unavailable_and_file_cannot_replace_anchor() {
    let f = Fixture::new();
    let mut broker = f.broker();
    assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
    f.write("notes", b"not a directory");
    assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
}

#[test]
fn symlink_hardlink_fifo_and_socket_are_errors_not_missing() {
    for attack in 0..4 {
        let f = Fixture::new();
        let mut broker = f.broker();
        f.write("notes/target", b"original");
        let path = f.root.join("notes/target");
        if attack != 1 {
            fs::remove_file(&path).unwrap();
        }
        let _socket = match attack {
            0 => {
                symlink("/etc/passwd", &path).unwrap();
                None
            }
            1 => {
                fs::hard_link(&path, f.root.join("notes/alias")).unwrap();
                None
            }
            2 => {
                assert!(
                    std::process::Command::new("/usr/bin/mkfifo")
                        .arg(&path)
                        .status()
                        .unwrap()
                        .success()
                );
                None
            }
            _ => Some(std::os::unix::net::UnixListener::bind(&path).unwrap()),
        };
        reset();
        assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
        assert_eq!(count(1), 0);
    }
}

#[test]
fn all_sibling_names_are_checked_before_exclusion_and_case_prefixes_are_fixed() {
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/good", b"ok");
    // 即使两个名字本来都被排除，也不能先过滤后隐藏 Unicode 别名。
    f.write("notes/private/unused", b"unused");
    match fs::create_dir(f.root.join("notes/PRIVATE")) {
        Ok(()) => {}
        Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists),
    }
    if fs::read_dir(f.root.join("notes"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("private")
        })
        .count()
        > 1
    {
        assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
    }
    // 不依赖文件系统支持两个别名：配置所选名字与实际目录大小写不同也拒绝。
    let mut changed = selection();
    changed.folders[0].path = path("NOTES");
    assert!(broker.capture_sync_observation(&changed, &[]).is_err());
}

#[test]
fn non_utf8_name_is_rejected_by_broker_or_by_the_macos_filesystem() {
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/good", b"ok");
    let target = f
        .root
        .join("notes")
        .join(std::ffi::OsString::from_vec(vec![0xff]));
    match fs::write(&target, b"bad") {
        Ok(()) => {
            reset();
            assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
            assert_eq!(count(1), 0);
        }
        Err(error) => {
            // APFS 在创建时返回 EILSEQ；Linux 必须实际造出该名字并验证扫描拒绝。
            #[cfg(target_os = "macos")]
            {
                assert_eq!(error.raw_os_error(), Some(92));
                assert!(!target.exists());
                assert_eq!(capture(&mut broker).entries().len(), 1);
            }
            #[cfg(not(target_os = "macos"))]
            panic!("non-UTF8 filename fixture must be constructible: {error}");
        }
    }
}

#[test]
fn invisible_and_non_nfc_names_fail_even_when_no_body_would_be_read() {
    for name in [
        std::ffi::OsString::from("hidden\u{202e}"),
        std::ffi::OsString::from("e\u{0301}"),
    ] {
        let f = Fixture::new();
        let mut broker = f.broker();
        f.write("notes/good", b"ok");
        fs::write(f.root.join("notes").join(name), b"bad").unwrap();
        reset();
        assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
        assert_eq!(count(1), 0);
    }
}

#[test]
fn overlapping_rules_and_file_directory_ambiguity_are_not_accepted() {
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/a", b"x");
    let mut overlap = selection();
    overlap.files.push(SyncReadRule {
        path: path("notes/a"),
        max_bytes: 1,
    });
    assert!(broker.capture_sync_observation(&overlap, &[]).is_err());
    assert!(
        broker
            .capture_sync_observation(&selection(), &[path("notes/a/child")])
            .is_err()
    );
}
