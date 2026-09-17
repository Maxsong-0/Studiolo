use super::fixture::{Fixture, directory};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn every_saved_root_replacement_invalidates_the_repository_approval() {
    for name in [
        "repository",
        "config",
        "auth",
        "audit",
        "vault",
        "assets",
        "writer",
        "ingress",
    ] {
        let f = Fixture::new();
        let approved = f.approve().unwrap();
        let path = f.root.join(name);
        fs::rename(&path, f.root.join(format!("old-{name}"))).unwrap();
        directory(&path);
        assert!(approved.revalidate().is_err(), "{name}");
    }
}

#[test]
fn private_root_permission_drift_and_config_same_length_change_are_rejected() {
    for name in ["config", "auth", "audit", "writer", "ingress"] {
        let f = Fixture::new();
        let approved = f.approve().unwrap();
        fs::set_permissions(f.root.join(name), fs::Permissions::from_mode(0o750)).unwrap();
        assert!(approved.revalidate().is_err(), "{name}");
    }
    let mut f = Fixture::new();
    let approved = f.approve().unwrap();
    let before = fs::read(&f.config).unwrap().len();
    f.document["project_repository"]["project"] = "项目/另一工坊".into();
    f.write();
    assert_eq!(fs::read(&f.config).unwrap().len(), before);
    assert!(approved.revalidate().is_err());
}

#[test]
fn ordinary_repository_content_updates_do_not_change_the_approved_root() {
    let f = Fixture::new();
    let approved = f.approve().unwrap();
    fs::write(
        f.repository.join("Cargo.toml"),
        "unparsed synthetic content",
    )
    .unwrap();
    directory(&f.repository.join("src"));
    approved.revalidate().unwrap();
}
