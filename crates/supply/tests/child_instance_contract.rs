//! 子实例升级、身份链与深度限制的持久契约。
use std::os::unix::fs::PermissionsExt;
use studiolo_supply::{AgentStore, SupplyError};

fn fixture() -> (tempfile::TempDir, AgentStore) {
    let temp = tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = AgentStore::open(temp.path()).unwrap();
    store.issue_type("claude", "2026-09-20T00:00:00Z").unwrap();
    store.issue_type("codex", "2026-09-20T00:00:00Z").unwrap();
    (temp, store)
}

#[test]
fn children_require_explicit_upgrade_and_survive_reopen() {
    let (temp, store) = fixture();
    let root = store
        .register_instance("claude", "root-session", "2026-09-20T00:00:00Z")
        .unwrap();
    assert!(matches!(
        store.register_child_instance("claude", &root.id, "child-session", "now"),
        Err(SupplyError::IdentityUpgradeRequired)
    ));
    store.upgrade_child_instances().unwrap();
    let child = store
        .register_child_instance("claude", &root.id, "child-session", "now")
        .unwrap();
    drop(store);
    let store = AgentStore::open_existing(temp.path()).unwrap();
    let chain = store.instance_lineage(&child.id).unwrap();
    assert_eq!(
        chain.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        vec![child.id.as_str(), root.id.as_str()]
    );
    assert!(store.upgrade_child_instances().is_err());
}

#[test]
fn parent_type_depth_and_missing_identity_are_closed() {
    let (_temp, store) = fixture();
    store.upgrade_child_instances().unwrap();
    let root = store
        .register_instance("claude", "root-session", "now")
        .unwrap();
    assert!(
        store
            .register_child_instance("codex", &root.id, "c", "now")
            .is_err()
    );
    assert!(
        store
            .register_child_instance("claude", "missing", "c", "now")
            .is_err()
    );
    let mut current = root;
    for depth in 1..=3 {
        current = store
            .register_child_instance("claude", &current.id, &format!("session-{depth}"), "now")
            .unwrap();
    }
    assert!(matches!(
        store.register_child_instance("claude", &current.id, "too-deep", "now"),
        Err(SupplyError::InvalidParent)
    ));
    store.revoke_type("claude", "later").unwrap();
    assert!(
        store
            .register_child_instance("claude", &current.id, "revoked", "later")
            .is_err()
    );
}
