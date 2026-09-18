//! 恢复契约：配置或根身份漂移后，既有能力必须立刻失效，不留部分可用状态。

use super::fixture::{Fixture, directory};
use serde_json::json;
use std::fs;

#[test]
fn capability_revalidation_fails_after_the_private_config_content_changes() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();
    assert_eq!(approved.revalidate(), Ok(()));

    let mut changed = fixture.value.clone();
    changed["sync_scope"]["scope_epoch"] = json!(2);
    fixture.write(&changed);

    assert!(approved.revalidate().is_err());
    assert!(scope.revalidate().is_err());
}

#[test]
fn capability_revalidation_fails_after_the_private_root_object_is_replaced() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();
    let pinned = approved.roots().private_identity();

    fs::remove_dir(fixture.root.join("transport")).unwrap();
    directory(&fixture.root.join("transport"));

    assert!(approved.revalidate().is_err());
    let reopened = fixture.approve(&scope).unwrap();
    assert_ne!(reopened.roots().private_identity(), pinned);
}

#[test]
fn capability_revalidation_fails_after_the_mirror_root_loses_private_permissions() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();

    fs::set_permissions(
        fixture.root.join("mirror"),
        <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o750),
    )
    .unwrap();

    assert!(approved.revalidate().is_err());
}
