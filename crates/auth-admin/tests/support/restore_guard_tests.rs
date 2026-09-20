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
