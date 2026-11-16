//! 身份只读视图：真实 SQLite 跨连接可见性、格式与路径身份检查。
use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_supply::{AgentIdentityView, AgentStore, SupplyError};
const NOW: &str = "2026-11-13T00:00:00Z";
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let path = root.path().join("agent");
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    (root, path)
}
#[test]
fn views_see_writer_updates_and_explicit_upgrade_without_exposing_digests() {
    let (_root, path) = fixture();
    let writer = AgentStore::open(&path).unwrap();
    writer.issue_type("claude", NOW).unwrap();
    let view = AgentIdentityView::open_existing(&path).unwrap();
    assert!(!view.children_enabled().unwrap());
    let parent = writer
        .register_instance("claude", "private-session", NOW)
        .unwrap();
    let rows = view.instances_with_parents(500).unwrap();
    assert_eq!(rows[0].instance, parent);
    assert_eq!(rows[0].parent, None);
    assert!(!format!("{rows:?}").contains("private-session"));
    writer.upgrade_child_instances().unwrap();
    let child = writer
        .register_child_instance("claude", &parent.id, "secret-child", "2026-11-13T01:00:00Z")
        .unwrap();
    writer
        .touch_instance(&child.id, "2026-11-13T02:00:00Z")
        .unwrap();
    let snapshot = view.snapshot(1).unwrap();
    assert!(snapshot.children_enabled);
    assert_eq!(
        snapshot.instances[0].parent.as_deref(),
        Some(parent.id.as_str())
    );
    assert_eq!(snapshot.lineages[&child.id].len(), 2);
    assert_eq!(snapshot.instances[0].last_seen_at, "2026-11-13T02:00:00Z");
    writer.revoke_type("claude", NOW).unwrap();
    assert_eq!(view.types().unwrap()[0].revoked_at.as_deref(), Some(NOW));
    assert!(!view.type_is_active("claude").unwrap());
}
#[test]
fn missing_unknown_format_and_replaced_directory_fail_closed() {
    let (root, path) = fixture();
    assert!(matches!(
        AgentIdentityView::open_existing(&path),
        Err(SupplyError::MissingStore)
    ));
    assert!(!path.join("agent.sqlite3").exists());
    let writer = AgentStore::open(&path).unwrap();
    let view = AgentIdentityView::open_existing(&path).unwrap();
    let raw = rusqlite::Connection::open(path.join("agent.sqlite3")).unwrap();
    raw.pragma_update(None, "user_version", 99).unwrap();
    assert!(matches!(
        AgentIdentityView::open_existing(&path),
        Err(SupplyError::CorruptStore)
    ));
    assert!(matches!(view.snapshot(1), Err(SupplyError::CorruptStore)));
    drop(raw);
    drop(writer);
    fs::rename(&path, root.path().join("old-agent")).unwrap();
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(view.snapshot(1), Err(SupplyError::UnsafeStorage)));
}

#[test]
fn isolated_view_rejects_content_static_private_roots_and_aliases() {
    let (root, identity) = fixture();
    let writer = AgentStore::open(&identity).unwrap();
    for name in ["vault", "web", "auth", "policy"] {
        fs::create_dir(root.path().join(name)).unwrap();
    }
    let isolated = [
        root.path().join("vault"),
        root.path().join("web"),
        root.path().join("auth"),
        root.path().join("policy"),
    ];
    let isolated: Vec<_> = isolated.iter().map(std::path::PathBuf::as_path).collect();
    assert!(AgentIdentityView::open_existing_isolated(&identity, &isolated).is_ok());
    assert!(AgentIdentityView::open_existing_isolated(&identity, &[root.path()]).is_err());
    assert!(AgentIdentityView::open_existing_isolated(&identity, &[&identity]).is_err());
    let nested = identity.join("served");
    fs::create_dir(&nested).unwrap();
    assert!(AgentIdentityView::open_existing_isolated(&identity, &[&nested]).is_err());
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&identity, &alias).unwrap();
    assert!(AgentIdentityView::open_existing_isolated(&alias, &isolated).is_err());
    assert!(AgentIdentityView::open_existing_isolated(&identity, &[&alias]).is_err());
    let traversal = root.path().join("vault/../agent");
    assert!(AgentIdentityView::open_existing_isolated(&identity, &[&traversal]).is_err());
    let similar = root.path().join("agent-public");
    fs::create_dir(&similar).unwrap();
    assert!(AgentIdentityView::open_existing_isolated(&identity, &[&similar]).is_ok());
    drop(writer);
}
