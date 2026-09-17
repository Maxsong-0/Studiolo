use super::{
    super::{MaintenanceError, inputs::Inputs, options::Command},
    fixture::Fixture,
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

#[test]
fn missing_writer_is_not_initialized_and_auth_store_is_not_opened() {
    let fixture = Fixture::new(false, false);
    assert!(matches!(
        Inputs::open(&fixture.options(Command::Inspect)),
        Err(MaintenanceError::Broker)
    ));
    for directory in ["data", "auth", "audit"] {
        assert_eq!(
            fs::read_dir(fixture.root.join(directory)).unwrap().count(),
            0
        );
    }
    assert!(!fixture.root.join("vault/.workbench").exists());
}

#[test]
fn inspect_can_open_broker_without_creating_a_missing_audit_store() {
    let fixture = Fixture::new(false, true);
    let inputs = Inputs::open(&fixture.options(Command::Inspect)).unwrap();
    assert!(inputs.audit.is_none());
    assert_eq!(fs::read_dir(fixture.root.join("audit")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(fixture.root.join("auth")).unwrap().count(), 0);
    inputs.revalidate().unwrap();
}

#[test]
fn actual_owner_lease_excludes_a_second_maintenance_instance() {
    let fixture = Fixture::new(true, true);
    let first = Inputs::open(&fixture.options(Command::Inspect)).unwrap();
    assert!(matches!(
        Inputs::open(&fixture.options(Command::Inspect)),
        Err(MaintenanceError::Broker)
    ));
    drop(first);
    assert!(Inputs::open(&fixture.options(Command::Inspect)).is_ok());
}

#[test]
fn root_or_config_drift_invalidates_the_existing_approval() {
    for name in ["vault", "data", "assets", "auth", "audit"] {
        let fixture = Fixture::new(false, true);
        let inputs = Inputs::open(&fixture.options(Command::Inspect)).unwrap();
        let path = fixture.root.join(name);
        fs::rename(&path, fixture.root.join(format!("{name}-old"))).unwrap();
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(inputs.revalidate().is_err());
    }
    let fixture = Fixture::new(false, true);
    let inputs = Inputs::open(&fixture.options(Command::Inspect)).unwrap();
    let mut bytes = fs::read(&fixture.config).unwrap();
    bytes.push(b' ');
    fs::write(&fixture.config, bytes).unwrap();
    assert!(inputs.revalidate().is_err());
}

#[test]
fn unsafe_config_links_and_overlapping_roots_fail_before_store_open() {
    let fixture = Fixture::new(false, false);
    let mut options = fixture.options(Command::Inspect);
    options.writer_data = options.vault.clone();
    assert!(matches!(
        Inputs::open(&options),
        Err(MaintenanceError::Configuration)
    ));
    let linked = fixture.root.join("config/link.json");
    symlink(&fixture.config, &linked).unwrap();
    let mut options = fixture.options(Command::Inspect);
    options.config = linked;
    assert!(matches!(
        Inputs::open(&options),
        Err(MaintenanceError::Configuration)
    ));
    fs::set_permissions(&fixture.config, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        Inputs::open(&fixture.options(Command::Inspect)),
        Err(MaintenanceError::Configuration)
    ));
    assert_eq!(fs::read_dir(fixture.root.join("data")).unwrap().count(), 0);
}
