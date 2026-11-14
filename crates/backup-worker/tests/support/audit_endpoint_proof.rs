use super::*;
use std::{
    fs as stdfs,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    time::Duration,
};
struct Fixture {
    root: tempfile::TempDir,
    _listener: UnixListener,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        stdfs::set_permissions(root.path(), stdfs::Permissions::from_mode(0o700)).unwrap();
        stdfs::create_dir(root.path().join("run")).unwrap();
        stdfs::set_permissions(
            root.path().join("run"),
            stdfs::Permissions::from_mode(0o750),
        )
        .unwrap();
        let listener = UnixListener::bind(root.path().join("run/audit.sock")).unwrap();
        stdfs::set_permissions(
            root.path().join("run/audit.sock"),
            stdfs::Permissions::from_mode(0o660),
        )
        .unwrap();
        Self {
            root,
            _listener: listener,
        }
    }
    fn proof(&self) -> Result<Proof, AuditError> {
        // 夹具以私有根 FD 开始，绕过 /tmp 的公共祖先；生产 open 始终从 / 开始。
        Proof::descend(
            fs::open(self.root.path(), FLAGS, Mode::empty()).unwrap(),
            &[OsStr::new("run"), OsStr::new("audit.sock")],
            rustix::process::geteuid().as_raw(),
            rustix::process::getegid().as_raw(),
            Instant::now() + Duration::from_secs(2),
        )
    }
}
#[test]
fn original_parent_and_socket_are_pinned_and_replacements_fail() {
    for swap_parent in [false, true] {
        let f = Fixture::new();
        let original = f.proof().unwrap();
        assert!(original.same(&f.proof().unwrap()).is_ok());
        if swap_parent {
            stdfs::rename(f.root.path().join("run"), f.root.path().join("old-run")).unwrap();
            stdfs::create_dir(f.root.path().join("run")).unwrap();
            stdfs::set_permissions(
                f.root.path().join("run"),
                stdfs::Permissions::from_mode(0o750),
            )
            .unwrap();
        } else {
            stdfs::rename(
                f.root.path().join("run/audit.sock"),
                f.root.path().join("run/old.sock"),
            )
            .unwrap();
        }
        let _new = UnixListener::bind(f.root.path().join("run/audit.sock")).unwrap();
        stdfs::set_permissions(
            f.root.path().join("run/audit.sock"),
            stdfs::Permissions::from_mode(0o660),
        )
        .unwrap();
        assert!(original.same(&f.proof().unwrap()).is_err());
    }
}
#[test]
fn unsafe_modes_wrong_groups_links_and_regular_files_cannot_be_endpoints() {
    for (name, mode) in [
        ("", 0o777),
        ("run", 0o770),
        ("run/audit.sock", 0o666),
        ("run/audit.sock", 0o600),
    ] {
        let f = Fixture::new();
        stdfs::set_permissions(
            f.root.path().join(name),
            stdfs::Permissions::from_mode(mode),
        )
        .unwrap();
        assert!(f.proof().is_err());
    }
    for directory in [false, true] {
        let f = Fixture::new();
        let relative = if directory { "run" } else { "run/audit.sock" };
        stdfs::rename(f.root.path().join(relative), f.root.path().join("kept")).unwrap();
        std::os::unix::fs::symlink(f.root.path().join("kept"), f.root.path().join(relative))
            .unwrap();
        assert!(f.proof().is_err());
    }
    let f = Fixture::new();
    stdfs::remove_file(f.root.path().join("run/audit.sock")).unwrap();
    stdfs::write(f.root.path().join("run/audit.sock"), b"not a socket").unwrap();
    stdfs::set_permissions(
        f.root.path().join("run/audit.sock"),
        stdfs::Permissions::from_mode(0o660),
    )
    .unwrap();
    assert!(f.proof().is_err());
}
#[test]
fn extended_acl_is_rejected_even_when_mode_bits_match_the_approved_layout() {
    for socket in [false, true] {
        let f = Fixture::new();
        let path = f
            .root
            .path()
            .join(if socket { "run/audit.sock" } else { "run" });
        let mode = if socket { 6_u16 } else { 5_u16 };
        let mut acl = 2_u32.to_le_bytes().to_vec();
        for (tag, perms, id) in [
            (1_u16, if socket { 6_u16 } else { 7_u16 }, u32::MAX),
            // 沙盒只映射当前 UID；额外命名条目仍构成扩展 ACL，不能用未映射 UID 构造夹具。
            (2, 7, rustix::process::geteuid().as_raw()),
            (4, mode, u32::MAX),
            (16, mode, u32::MAX),
            (32, 0, u32::MAX),
        ] {
            acl.extend(tag.to_le_bytes());
            acl.extend(perms.to_le_bytes());
            acl.extend(id.to_le_bytes());
        }
        fs::setxattr(
            &path,
            "system.posix_acl_access",
            &acl,
            fs::XattrFlags::empty(),
        )
        .unwrap();
        assert!(f.proof().is_err());
    }
}
