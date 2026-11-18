//! 无需 root 即覆盖封闭布局和运行上下文判定；不读取真实恢复数据。
use super::*;

#[test]
fn context_requires_linux_real_and_effective_root_private_stdin_and_netns() {
    assert!(context(true, 0, 0, false, true).is_ok());
    for (linux, real, effective, tty, separate) in [
        (false, 0, 0, false, true),
        (true, 1, 0, false, true),
        (true, 0, 1, false, true),
        (true, 0, 0, true, true),
        (true, 0, 0, false, false),
    ] {
        assert!(context(linux, real, effective, tty, separate).is_err());
    }
}

#[test]
fn only_the_exact_isolated_layout_is_accepted() {
    let root = Path::new("/var/lib/studiolo-dr-tests/fictional-1/human-work");
    assert!(layout(root, &root.join("owner.json")).is_ok());
    for value in [
        "/var/lib/studiolo-human",
        "/var/lib/studiolo-dr-tests/x/human-work/",
        "/var/lib/studiolo-dr-tests/x/../human-work",
        "/var/lib/studiolo-dr-tests/a_b/human-work",
        "/var/lib/studiolo-dr-tests//human-work",
        "/var/lib/studiolo-dr-tests/x/human-work/other",
        "/var/lib/studiolo-dr-tests/秘密/human-work",
        "relative",
    ] {
        let path = Path::new(value);
        assert!(layout(path, &path.join("owner.json")).is_err());
    }
    let long = PathBuf::from(format!("{BASE}/{}/human-work", "a".repeat(65)));
    assert!(layout(&long, &long.join("owner.json")).is_err());
    assert!(layout(root, &root.join("other.json")).is_err());
    assert!(layout(root, &root.join("./owner.json")).is_err());
}

#[test]
fn permission_rules_reject_other_owners_links_and_public_private_nodes() {
    for kind in [Kind::Database, Kind::Config] {
        assert!(permissions(0o600, 0, 1, kind));
        for (mode, uid, links) in [
            (0o644, 0, 1),
            (0o600, 1000, 1),
            (0o600, 0, 2),
            (0o4600, 0, 1),
        ] {
            assert!(!permissions(mode, uid, links, kind));
        }
    }
    assert!(permissions(0o700, 0, 2, Kind::Directory));
    assert!(!permissions(0o755, 0, 2, Kind::Directory));
    assert!(!permissions(0o700, 0, 0, Kind::Directory));
    assert!(!permissions(0o775, 0, 2, Kind::Ancestor));
    assert!(!permissions(0o1777, 0, 2, Kind::Ancestor));
    assert!(!permissions(0o1777, 1000, 2, Kind::Ancestor));
}

#[test]
fn held_file_checks_reject_symlinks_special_nodes_and_other_devices() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("fixture");
    fs::write(&path, b"fixture").unwrap();
    let metadata = fs::symlink_metadata(&path).unwrap();
    assert!(inspect(&metadata, Kind::Config, metadata.dev().wrapping_add(1)).is_err());
    std::os::unix::fs::symlink(&path, temporary.path().join("link")).unwrap();
    let linked = fs::symlink_metadata(temporary.path().join("link")).unwrap();
    assert!(inspect(&linked, Kind::Config, linked.dev()).is_err());
    let directory = fs::symlink_metadata(temporary.path()).unwrap();
    assert!(inspect(&directory, Kind::Database, directory.dev()).is_err());
}

#[test]
fn only_the_audit_directory_admits_the_operation_gate() {
    use std::ffi::OsStr;
    assert_eq!(
        entry_kind("audit", OsStr::new(AUDIT_GATE)),
        Some(Kind::Gate)
    );
    assert_eq!(entry_kind("auth", OsStr::new(AUDIT_GATE)), None);
    for directory in ["auth", "audit"] {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let name = format!("{directory}.sqlite3{suffix}");
            assert_eq!(
                entry_kind(directory, OsStr::new(&name)),
                Some(Kind::Database)
            );
        }
    }
    for name in [
        "checkpoint-operations.lock.tmp",
        "notes.md",
        "auth.sqlite3",
        "audit.sqlite3-other",
    ] {
        assert_eq!(entry_kind("audit", OsStr::new(name)), None, "{name}");
    }
}

#[test]
fn operation_gate_must_be_a_private_empty_single_link_file() {
    assert!(permissions(0o600, 0, 1, Kind::Gate));
    for (mode, uid, links) in [
        (0o644, 0, 1),
        (0o600, 1000, 1),
        (0o600, 0, 2),
        (0o4600, 0, 1),
    ] {
        assert!(!permissions(mode, uid, links, Kind::Gate));
    }
    assert!(size_allowed(Kind::Gate, 0));
    assert!(!size_allowed(Kind::Gate, 1));
    assert!(size_allowed(Kind::Database, 4096));
}

#[test]
fn every_name_a_freshly_opened_audit_ledger_leaves_is_admitted() {
    // 真实 ledger 开库时写者会立即取一次操作许可、建准入锁；恢复流程在开库之后还要复核布局，
    // 这里核对 ledger 留下的每个名字都在恢复守卫的白名单里，名字一改两边对不上就红。
    use std::os::unix::fs::PermissionsExt;
    let temporary = tempfile::tempdir().unwrap();
    let audit = temporary.path().join("audit");
    fs::create_dir(&audit).unwrap();
    fs::set_permissions(&audit, fs::Permissions::from_mode(0o700)).unwrap();
    let ledger = studiolo_audit_ledger::SqliteAuditLedger::open(&audit).unwrap();
    let names = fs::read_dir(&audit)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert!(names.iter().any(|name| name == AUDIT_GATE), "{names:?}");
    for name in &names {
        assert!(entry_kind("audit", name).is_some(), "{name:?}");
    }
    drop(ledger);
}
