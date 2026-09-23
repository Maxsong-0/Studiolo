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
            matches!(
                broker.capture_sync_observation(&selection(), &[path(name)]),
                Err(BrokerError::InvalidArtifact(reason)) if reason == INVALID
            ),
            "{name}"
        );
        assert_eq!(count(0), 0);
        assert_eq!(count(1), 0);
    }
}

#[test]
fn selected_missing_anchor_is_unavailable_and_file_cannot_replace_anchor() {
    let f = Fixture::new();
    let mut broker = f.broker();
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &[]),
        Err(BrokerError::NotFound(anchor)) if anchor == path("notes")
    ));
    f.write("notes", b"not a directory");
    // 所选目录锚点是普通文件：属于文件/目录歧义，按非法选择拒绝而不是当作缺失。
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &[]),
        Err(BrokerError::InvalidArtifact(reason)) if reason == INVALID
    ));
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
        // 链接按链接拒绝；硬链接、FIFO 与 socket 都不是唯一普通文件。硬链接两个名字谁先
        // 被枚举取决于目录顺序，只断言拒绝原因。
        let expected = if attack == 0 {
            "linked or cross-device sync object"
        } else {
            "sync observation requires unique regular file"
        };
        let result = broker.capture_sync_observation(&selection(), &[]);
        assert!(
            matches!(&result, Err(BrokerError::UnsafeNode { reason, .. }) if *reason == expected),
            "attack {attack}: {:?}",
            result.err()
        );
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
    let mut aliases = fs::read_dir(f.root.join("notes"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.eq_ignore_ascii_case("private"))
        .collect::<Vec<_>>();
    aliases.sort();
    // 分支自证：卷是否区分大小写决定走哪一支；打印判据原始输入（实际并存的拼写）。
    eprintln!("同名异写目录·分支自证：notes 下 private 的实际拼写 = {aliases:?}");
    if aliases.len() > 1 {
        // 排除前缀 notes/private 在选择校验时已入索引，与目录顺序无关，磁盘上的 PRIVATE
        // 总是与它碰撞。
        assert!(matches!(
            broker.capture_sync_observation(&selection(), &[]),
            Err(BrokerError::PathCollision { requested, existing })
                if requested == path("notes/PRIVATE") && existing == "notes/private"
        ));
    } else {
        eprintln!("跳过：临时卷不区分大小写，PRIVATE 落到既有 private；改验排除子树不影响捕获");
        assert_eq!(aliases, ["private"]);
        let observed = capture(&mut broker);
        assert_eq!(
            observed.entries().keys().cloned().collect::<Vec<_>>(),
            [path("notes/good")]
        );
    }
    // 不依赖文件系统支持两个别名：配置所选名字与实际目录大小写不同也拒绝；这是对选择
    // 本身的检查，在接触文件系统之前完成。
    let mut changed = selection();
    changed.folders[0].path = path("NOTES");
    assert!(matches!(
        broker.capture_sync_observation(&changed, &[]),
        Err(BrokerError::PathCollision { requested, existing })
            if requested == path("notes/generated.md") && existing == "NOTES"
    ));
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
    let created = fs::write(&target, b"bad");
    // 分支自证：文件系统是否接受非 UTF-8 名字决定走哪一支；打印判据原始输入。
    eprintln!("非 UTF-8 名字·分支自证：创建结果 = {created:?}");
    match created {
        Ok(()) => {
            reset();
            assert!(matches!(
                broker.capture_sync_observation(&selection(), &[]),
                Err(BrokerError::InvalidArtifact(reason)) if reason == INVALID
            ));
            assert_eq!(count(1), 0);
        }
        Err(error) => {
            eprintln!("跳过：文件系统拒绝创建非 UTF-8 名字（{error}）；改验扫描结果不含该名字");
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
        fs::write(f.root.join("notes").join(&name), b"bad").unwrap();
        reset();
        assert!(
            matches!(
                broker.capture_sync_observation(&selection(), &[]),
                Err(BrokerError::InvalidArtifact(reason)) if reason == INVALID
            ),
            "{name:?}"
        );
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
    assert!(matches!(
        broker.capture_sync_observation(&overlap, &[]),
        Err(BrokerError::InvalidArtifact(reason)) if reason == INVALID
    ));
    // notes/a 实际是文件，要求其下的子路径时父链不成立，按捕获已变拒绝。
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &[path("notes/a/child")]),
        Err(BrokerError::CaptureChanged)
    ));
}
