//! Existing identity, zero creation, and rejection of changed canonical bindings.

use std::{fs, os::unix::fs::symlink, path::Path};
use studiolo_core::VaultUuid;
use studiolo_vault_broker::{BrokerError, CanonicalReadVault};

use super::support::{Fixture, create_fifo, inventory, path};

#[test]
fn opens_and_reads_without_creating_or_changing_any_source_or_data_file() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("note.md"), b"# existing\n").unwrap();
    let before = inventory(fixture.directory.path());
    let reader = fixture.reader();
    assert_eq!(
        reader.read_file(&path("note.md"), 64).unwrap().content,
        b"# existing\n"
    );
    drop(reader);
    assert_eq!(inventory(fixture.directory.path()), before);
}

#[test]
fn a_missing_root_or_identity_is_never_initialized() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("missing");
    assert!(CanonicalReadVault::open_existing(&root).is_err());
    assert!(!root.exists());
    fs::create_dir(&root).unwrap();
    let before = inventory(temp.path());
    assert!(matches!(
        CanonicalReadVault::open_existing(&root),
        Err(BrokerError::CorruptIdentity)
    ));
    assert_eq!(inventory(temp.path()), before);
    fs::create_dir(root.join(".workbench")).unwrap();
    let before = inventory(temp.path());
    assert!(matches!(
        CanonicalReadVault::open_existing(&root),
        Err(BrokerError::CorruptIdentity)
    ));
    assert_eq!(inventory(temp.path()), before);
    assert!(CanonicalReadVault::open_existing(Path::new("relative-vault")).is_err());
}

#[test]
fn invalid_or_oversized_identity_is_refused_without_repair() {
    let fixture = Fixture::new();
    for bytes in [
        b"not JSON".to_vec(),
        b"{}".to_vec(),
        b"{\"format_version\":1,\"vault_uuid\":\"invalid\"}".to_vec(),
        format!(
            "{{\"format_version\":2,\"vault_uuid\":\"{}\"}}",
            VaultUuid::new()
        )
        .into_bytes(),
        format!(
            "{{\"format_version\":1,\"format_version\":1,\"vault_uuid\":\"{}\"}}",
            VaultUuid::new()
        )
        .into_bytes(),
        vec![b' '; 4097],
    ] {
        fs::write(fixture.marker(), &bytes).unwrap();
        let before = inventory(fixture.directory.path());
        assert!(matches!(
            CanonicalReadVault::open_existing(&fixture.root),
            Err(BrokerError::CorruptIdentity)
        ));
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}

#[test]
fn identity_rebinding_removal_and_corruption_are_checked_on_every_read() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("note.md"), b"content").unwrap();
    let reader = fixture.reader();
    let old_identity = fs::read(fixture.marker()).unwrap();
    let changed = format!(
        "{{\"format_version\":1,\"vault_uuid\":\"{}\"}}",
        VaultUuid::new()
    );
    for bytes in [changed.as_bytes(), b"bad identity"] {
        fs::write(fixture.marker(), bytes).unwrap();
        assert!(matches!(
            reader.read_file(&path("note.md"), 64),
            Err(BrokerError::CorruptIdentity)
        ));
        assert!(matches!(
            reader.root_identity(),
            Err(BrokerError::CorruptIdentity)
        ));
    }
    fs::remove_file(fixture.marker()).unwrap();
    assert!(matches!(
        reader.read_file(&path("note.md"), 64),
        Err(BrokerError::CorruptIdentity)
    ));
    fs::write(fixture.marker(), old_identity).unwrap();
    assert_eq!(
        reader.read_file(&path("note.md"), 64).unwrap().content,
        b"content"
    );
}

#[test]
fn root_replacement_is_rejected_even_when_the_replacement_copies_the_uuid() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("note.md"), b"original").unwrap();
    let reader = fixture.reader();
    let root_identity = reader.root_identity().unwrap();
    let marker = fs::read(fixture.marker()).unwrap();
    fs::rename(&fixture.root, fixture.directory.path().join("retained")).unwrap();
    fs::create_dir_all(fixture.root.join(".workbench")).unwrap();
    fs::write(fixture.marker(), marker).unwrap();
    fs::write(fixture.root.join("note.md"), b"replacement").unwrap();
    assert!(matches!(
        reader.read_file(&path("note.md"), 64),
        Err(BrokerError::RootChanged)
    ));
    assert!(matches!(
        reader.root_identity(),
        Err(BrokerError::RootChanged)
    ));
    assert_ne!(fixture.reader().root_identity().unwrap(), root_identity);
}

#[test]
fn symlink_roots_and_identity_links_are_never_followed() {
    let fixture = Fixture::new();
    let alias = fixture.directory.path().join("alias");
    symlink(&fixture.root, &alias).unwrap();
    for root in [
        alias.clone(),
        alias.join("."),
        Path::new(&format!("{}/", alias.display())).to_owned(),
    ] {
        assert!(CanonicalReadVault::open_existing(&root).is_err());
    }
    let marker_copy = fixture.directory.path().join("identity-copy");
    fs::rename(fixture.marker(), &marker_copy).unwrap();
    symlink(&marker_copy, fixture.marker()).unwrap();
    assert!(matches!(
        CanonicalReadVault::open_existing(&fixture.root),
        Err(BrokerError::CorruptIdentity)
    ));
    fs::remove_file(fixture.marker()).unwrap();
    fs::hard_link(&marker_copy, fixture.marker()).unwrap();
    assert!(matches!(
        CanonicalReadVault::open_existing(&fixture.root),
        Err(BrokerError::CorruptIdentity)
    ));
}

#[test]
fn missing_or_linked_selected_root_is_refused_after_open() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("note.md"), b"original").unwrap();
    let reader = fixture.reader();
    let retained = fixture.directory.path().join("retained");
    fs::rename(&fixture.root, &retained).unwrap();
    assert!(matches!(
        reader.read_file(&path("note.md"), 64),
        Err(BrokerError::RootChanged)
    ));
    symlink(&retained, &fixture.root).unwrap();
    assert!(matches!(
        reader.read_file(&path("note.md"), 64),
        Err(BrokerError::RootChanged)
    ));
}

#[test]
fn linked_control_directory_and_special_identity_files_are_refused() {
    let fixture = Fixture::new();
    let control = fixture.root.join(".workbench");
    let retained = fixture.directory.path().join("retained-control");
    fs::rename(&control, &retained).unwrap();
    symlink(&retained, &control).unwrap();
    assert!(matches!(
        CanonicalReadVault::open_existing(&fixture.root),
        Err(BrokerError::CorruptIdentity)
    ));
    fs::remove_file(&control).unwrap();
    fs::create_dir(&control).unwrap();
    create_fifo(&fixture.marker());
    assert!(matches!(
        CanonicalReadVault::open_existing(&fixture.root),
        Err(BrokerError::CorruptIdentity)
    ));
    fs::remove_file(fixture.marker()).unwrap();
    fs::create_dir(fixture.marker()).unwrap();
    assert!(matches!(
        CanonicalReadVault::open_existing(&fixture.root),
        Err(BrokerError::CorruptIdentity)
    ));
}
