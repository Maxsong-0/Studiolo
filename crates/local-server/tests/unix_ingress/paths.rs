use super::support::{Fixture, mode};
use std::{
    fs,
    os::unix::{fs::symlink, net::UnixListener},
    path::Path,
};
use studiolo_local_server::PreparedUnixIngress;

#[test]
fn preparation_is_read_only_and_requires_an_existing_private_parent() {
    let fixture = Fixture::new();
    let prepared = PreparedUnixIngress::prepare(&fixture.socket, &[&fixture.isolated]).unwrap();
    prepared.revalidate().unwrap();
    assert_eq!(fs::read_dir(&fixture.parent).unwrap().count(), 0);
    let missing = fixture.root.join("missing/http.sock");
    assert!(PreparedUnixIngress::prepare(&missing, &[]).is_err());
    assert!(!missing.parent().unwrap().exists());
    for permissions in [0o755, 0o770, 0o1700, 0o600] {
        mode(&fixture.parent, permissions);
        assert!(PreparedUnixIngress::prepare(&fixture.socket, &[]).is_err());
    }
    mode(&fixture.parent, 0o700);
}

#[test]
fn rejects_noncanonical_long_and_root_paths_without_disclosing_them() {
    let fixture = Fixture::new();
    let paths = [
        "".to_string(),
        "http.sock".into(),
        "/".into(),
        format!("{}/./http.sock", fixture.parent.display()),
        format!("{}//http.sock", fixture.parent.display()),
        format!("{}/../http.sock", fixture.parent.display()),
        format!("{}/http.sock/", fixture.parent.display()),
        format!("{}/bad\n.sock", fixture.parent.display()),
        format!("{}/{}", fixture.parent.display(), "x".repeat(104)),
    ];
    for path in paths {
        let error = PreparedUnixIngress::prepare(Path::new(&path), &[])
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "Unix 入口路径或权限无效");
    }
}

#[test]
fn refuses_every_existing_node_without_unlinking() {
    let fixture = Fixture::new();
    fs::write(&fixture.socket, "keep-file").unwrap();
    assert!(PreparedUnixIngress::prepare(&fixture.socket, &[]).is_err());
    assert_eq!(fs::read_to_string(&fixture.socket).unwrap(), "keep-file");
    fs::remove_file(&fixture.socket).unwrap();
    fs::create_dir(&fixture.socket).unwrap();
    assert!(PreparedUnixIngress::prepare(&fixture.socket, &[]).is_err());
    assert!(fixture.socket.is_dir());
    fs::remove_dir(&fixture.socket).unwrap();
    symlink(fixture.root.join("missing"), &fixture.socket).unwrap();
    assert!(PreparedUnixIngress::prepare(&fixture.socket, &[]).is_err());
    assert!(
        fs::symlink_metadata(&fixture.socket)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::remove_file(&fixture.socket).unwrap();
    let _listener = UnixListener::bind(&fixture.socket).unwrap();
    assert!(PreparedUnixIngress::prepare(&fixture.socket, &[]).is_err());
    assert!(fixture.socket.exists());
}

#[test]
fn refuses_fifo_without_opening_it() {
    let fixture = Fixture::new();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fixture.socket)
            .status()
            .unwrap()
            .success()
    );
    assert!(PreparedUnixIngress::prepare(&fixture.socket, &[]).is_err());
    assert!(fs::symlink_metadata(&fixture.socket).is_ok());
}

#[test]
fn rejects_links_in_any_parent_or_isolated_root() {
    let fixture = Fixture::new();
    let alias = fixture.root.join("alias");
    symlink(&fixture.parent, &alias).unwrap();
    assert!(PreparedUnixIngress::prepare(&alias.join("http.sock"), &[]).is_err());
    let root_alias = fixture.root.join("data-alias");
    symlink(&fixture.isolated, &root_alias).unwrap();
    assert!(PreparedUnixIngress::prepare(&fixture.socket, &[&root_alias]).is_err());
    let ancestor_alias = fixture.root.join("ancestor-alias");
    symlink(&fixture.root, &ancestor_alias).unwrap();
    assert!(PreparedUnixIngress::prepare(&ancestor_alias.join("run/http.sock"), &[]).is_err());
}

#[test]
fn rejects_equal_or_nested_roots_in_both_directions() {
    let fixture = Fixture::new();
    let nested = fixture.parent.join("nested");
    fs::create_dir(&nested).unwrap();
    for root in [&fixture.parent, &fixture.root, &nested] {
        assert!(PreparedUnixIngress::prepare(&fixture.socket, &[root]).is_err());
    }
    PreparedUnixIngress::prepare(&fixture.socket, &[&fixture.isolated]).unwrap();
}

#[test]
fn case_aliases_use_real_directory_identity_when_the_filesystem_has_them() {
    let fixture = Fixture::new();
    let alias = fixture.root.join("RUN");
    // Linux 默认大小写敏感；macOS 也可使用大小写敏感卷，不虚报别名证据。
    if alias.exists() {
        assert!(PreparedUnixIngress::prepare(&fixture.socket, &[&alias]).is_err());
    } else {
        fs::create_dir(&alias).unwrap();
        PreparedUnixIngress::prepare(&fixture.socket, &[&alias]).unwrap();
    }
}
