use super::support::{Fixture, mode};
use std::{
    fs,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, symlink},
        net::UnixListener,
    },
};
use studiolo_local_server::PreparedUnixIngress;

#[test]
fn bind_needs_no_runtime_and_drop_removes_its_own_private_socket() {
    let fixture = Fixture::new();
    let bound = PreparedUnixIngress::prepare(&fixture.socket, &[&fixture.isolated])
        .unwrap()
        .bind()
        .unwrap();
    let metadata = fs::symlink_metadata(&fixture.socket).unwrap();
    assert!(metadata.file_type().is_socket());
    assert_eq!(metadata.mode() & 0o7777, 0o600);
    assert_eq!(metadata.uid(), rustix::process::getuid().as_raw());
    drop(bound);
    assert!(!fixture.socket.exists());
}

#[test]
fn prepared_identity_and_permissions_are_rechecked_before_bind() {
    let fixture = Fixture::new();
    let prepared = PreparedUnixIngress::prepare(&fixture.socket, &[&fixture.isolated]).unwrap();
    mode(&fixture.parent, 0o755);
    assert!(prepared.revalidate().is_err());
    assert!(prepared.bind().is_err());
    assert!(!fixture.socket.exists());
    mode(&fixture.parent, 0o700);
    let prepared = PreparedUnixIngress::prepare(&fixture.socket, &[&fixture.isolated]).unwrap();
    fs::rename(&fixture.parent, fixture.root.join("old-run")).unwrap();
    fs::create_dir(&fixture.parent).unwrap();
    mode(&fixture.parent, 0o700);
    assert!(prepared.revalidate().is_err());
    assert!(prepared.bind().is_err());
    assert!(!fixture.socket.exists());
}

#[test]
fn isolated_root_replacement_and_new_target_are_rejected() {
    let fixture = Fixture::new();
    let prepared = PreparedUnixIngress::prepare(&fixture.socket, &[&fixture.isolated]).unwrap();
    fs::rename(&fixture.isolated, fixture.root.join("old-data")).unwrap();
    fs::create_dir(&fixture.isolated).unwrap();
    assert!(prepared.revalidate().is_err());
    assert!(prepared.bind().is_err());
    let prepared = PreparedUnixIngress::prepare(&fixture.socket, &[&fixture.isolated]).unwrap();
    fs::write(&fixture.socket, "arrived-after-prepare").unwrap();
    assert!(prepared.bind().is_err());
    assert_eq!(
        fs::read_to_string(&fixture.socket).unwrap(),
        "arrived-after-prepare"
    );
}

#[test]
fn cleanup_leaves_replacement_files_links_and_sockets_untouched() {
    for replacement in ["file", "link", "socket"] {
        let fixture = Fixture::new();
        let bound = PreparedUnixIngress::prepare(&fixture.socket, &[])
            .unwrap()
            .bind()
            .unwrap();
        fs::rename(&fixture.socket, fixture.parent.join("owned.sock")).unwrap();
        let target = fixture.root.join("target");
        fs::write(&target, "untouched").unwrap();
        mode(&target, 0o644);
        let listener = match replacement {
            "file" => {
                fs::write(&fixture.socket, "replacement").unwrap();
                None
            }
            "link" => {
                symlink(&target, &fixture.socket).unwrap();
                None
            }
            _ => Some(UnixListener::bind(&fixture.socket).unwrap()),
        };
        let before = fs::symlink_metadata(&fixture.socket).unwrap();
        drop(bound);
        let after = fs::symlink_metadata(&fixture.socket).unwrap();
        assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
        assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, 0o644);
        drop(listener);
    }
}

#[test]
fn cleanup_uses_original_parent_descriptor_after_path_replacement() {
    let fixture = Fixture::new();
    let bound = PreparedUnixIngress::prepare(&fixture.socket, &[])
        .unwrap()
        .bind()
        .unwrap();
    let old_parent = fixture.root.join("old-run");
    fs::rename(&fixture.parent, &old_parent).unwrap();
    fs::create_dir(&fixture.parent).unwrap();
    fs::write(&fixture.socket, "new-parent-file").unwrap();
    drop(bound);
    assert_eq!(
        fs::read_to_string(&fixture.socket).unwrap(),
        "new-parent-file"
    );
    assert!(!old_parent.join("http.sock").exists());
}

#[tokio::test]
async fn serve_rechecks_permission_and_cleans_on_startup_failure() {
    let fixture = Fixture::new();
    let bound = PreparedUnixIngress::prepare(&fixture.socket, &[])
        .unwrap()
        .bind()
        .unwrap();
    mode(&fixture.socket, 0o666);
    assert!(bound.serve(axum::Router::new()).await.is_err());
    assert!(!fixture.socket.exists());
}

#[test]
fn replacing_an_ancestor_is_detected_even_when_the_leaf_inode_is_preserved() {
    let fixture = Fixture::new();
    let ancestor = fixture.root.join("ancestor");
    fs::create_dir(&ancestor).unwrap();
    let parent = ancestor.join("run");
    fs::create_dir(&parent).unwrap();
    mode(&parent, 0o700);
    let socket = parent.join("http.sock");
    let prepared = PreparedUnixIngress::prepare(&socket, &[]).unwrap();
    let displaced = fixture.root.join("displaced");
    fs::rename(&ancestor, &displaced).unwrap();
    fs::create_dir(&ancestor).unwrap();
    fs::rename(displaced.join("run"), &parent).unwrap();
    assert!(prepared.revalidate().is_err());
    assert!(prepared.bind().is_err());
    assert!(!socket.exists());
}

#[tokio::test]
async fn serve_refuses_a_replaced_socket_without_touching_the_replacement() {
    let fixture = Fixture::new();
    let bound = PreparedUnixIngress::prepare(&fixture.socket, &[])
        .unwrap()
        .bind()
        .unwrap();
    fs::rename(&fixture.socket, fixture.parent.join("owned.sock")).unwrap();
    fs::write(&fixture.socket, "replacement").unwrap();
    assert!(bound.serve(axum::Router::new()).await.is_err());
    assert_eq!(fs::read_to_string(&fixture.socket).unwrap(), "replacement");
}
