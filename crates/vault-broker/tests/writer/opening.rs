use super::support::{Fixture, inventory};
use std::{fs, os::unix::fs::symlink};
use studiolo_vault_broker::{BrokerError, FsVaultBroker, VaultReader};

#[test]
fn explicit_initialization_requires_two_existing_empty_roots() {
    let fixture = Fixture::empty();
    assert!(FsVaultBroker::initialize_empty(&fixture.vault, &fixture.data).is_err());
    assert!(!fixture.vault.exists());
    assert!(!fixture.data.exists());
    fs::create_dir(&fixture.vault).unwrap();
    assert!(FsVaultBroker::initialize_empty(&fixture.vault, &fixture.data).is_err());
    assert!(inventory(&fixture.vault).is_empty());
    assert!(!fixture.data.exists());
    fs::create_dir(&fixture.data).unwrap();
    let writer = FsVaultBroker::initialize_empty(&fixture.vault, &fixture.data).unwrap();
    let uuid = writer.vault_uuid();
    drop(writer);
    assert_eq!(fixture.open().vault_uuid(), uuid);
    let before = inventory(fixture.directory.path());
    assert!(FsVaultBroker::initialize_empty(&fixture.vault, &fixture.data).is_err());
    assert_eq!(inventory(fixture.directory.path()), before);
}

#[test]
fn explicit_initialization_rejects_nonempty_roots_before_any_write() {
    for node in ["vault", "data"] {
        for name in ["note.md", ".hidden"] {
            let fixture = Fixture::empty();
            fs::create_dir(&fixture.vault).unwrap();
            fs::create_dir(&fixture.data).unwrap();
            let directory = if node == "vault" {
                &fixture.vault
            } else {
                &fixture.data
            };
            fs::write(directory.join(name), b"preserve original bytes").unwrap();
            let before = inventory(fixture.directory.path());
            assert!(FsVaultBroker::initialize_empty(&fixture.vault, &fixture.data).is_err());
            assert_eq!(inventory(fixture.directory.path()), before);
        }
    }
}

#[test]
fn explicit_initialization_obeys_existing_lease_and_path_boundaries() {
    let fixture = Fixture::initialized();
    let writer = fixture.open();
    assert!(matches!(
        FsVaultBroker::initialize_empty(&fixture.vault, &fixture.data),
        Err(BrokerError::WriterBusy)
    ));
    drop(writer);
    let fixture = Fixture::empty();
    fs::create_dir(&fixture.vault).unwrap();
    fs::create_dir(&fixture.data).unwrap();
    let link = fixture
        .directory
        .path()
        .canonicalize()
        .unwrap()
        .join("linked-data");
    symlink(&fixture.data, &link).unwrap();
    let before = inventory(fixture.directory.path());
    assert!(FsVaultBroker::initialize_empty(&fixture.vault, &link).is_err());
    assert!(FsVaultBroker::initialize_empty(&fixture.vault, &fixture.vault).is_err());
    assert!(
        FsVaultBroker::initialize_empty(&fixture.vault, &fixture.vault.join("nested")).is_err()
    );
    assert_eq!(inventory(fixture.directory.path()), before);
}

#[test]
fn existing_open_creates_neither_roots_nor_missing_identity() {
    let fixture = Fixture::empty();
    assert!(FsVaultBroker::open_existing(&fixture.vault, &fixture.data).is_err());
    assert!(!fixture.vault.exists());
    assert!(!fixture.data.exists());
    fs::create_dir(&fixture.vault).unwrap();
    fs::create_dir(&fixture.data).unwrap();
    let before = inventory(fixture.directory.path());
    assert!(FsVaultBroker::open_existing(&fixture.vault, &fixture.data).is_err());
    assert_eq!(inventory(fixture.directory.path()), before);
}

#[test]
fn existing_open_does_not_repair_removed_data_layout() {
    for directory in ["history", "idempotency", "snapshots"] {
        let fixture = Fixture::initialized();
        fs::remove_dir(fixture.data.join(directory)).unwrap();
        let before = inventory(fixture.directory.path());
        assert!(
            FsVaultBroker::open_existing(&fixture.vault, &fixture.data).is_err(),
            "repaired {directory}"
        );
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}

#[test]
fn paired_roots_cannot_be_crossed_or_replaced_by_an_empty_data_root() {
    let first = Fixture::initialized();
    let second = Fixture::initialized();
    assert_ne!(first.open().vault_uuid(), second.open().vault_uuid());
    let before_first = inventory(first.directory.path());
    let before_second = inventory(second.directory.path());
    assert!(matches!(
        FsVaultBroker::open_existing(&first.vault, &second.data),
        Err(BrokerError::WriterBindingMismatch)
    ));
    assert!(matches!(
        FsVaultBroker::open(&first.vault, &second.data),
        Err(BrokerError::WriterBindingMismatch)
    ));
    assert_eq!(inventory(first.directory.path()), before_first);
    assert_eq!(inventory(second.directory.path()), before_second);
    let other = first.data.with_file_name("empty-data");
    fs::create_dir(&other).unwrap();
    assert!(matches!(
        FsVaultBroker::open(&first.vault, &other),
        Err(BrokerError::WriterBindingMismatch)
    ));
    assert!(inventory(&other).is_empty());
    drop(first.open());
    drop(second.open());
}

#[test]
fn overlapping_canonical_and_data_roots_are_refused() {
    for arrangement in ["equal", "data-inside-vault", "vault-inside-data"] {
        let fixture = Fixture::empty();
        fs::create_dir(&fixture.vault).unwrap();
        let (vault, data) = match arrangement {
            "equal" => (fixture.vault.clone(), fixture.vault.clone()),
            "data-inside-vault" => (fixture.vault.clone(), fixture.vault.join("data")),
            _ => (fixture.vault.join("canonical"), fixture.vault.clone()),
        };
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&data).unwrap();
        let before = inventory(fixture.directory.path());
        assert!(
            FsVaultBroker::open(&vault, &data).is_err(),
            "accepted {arrangement}"
        );
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}

#[test]
fn actual_case_aliases_cannot_create_a_nested_data_root() {
    use std::os::unix::fs::MetadataExt;

    let fixture = Fixture::empty();
    fs::create_dir(&fixture.vault).unwrap();
    let alias = fixture.vault.with_file_name("VAULT");
    let original = fs::metadata(&fixture.vault).unwrap();
    match fs::metadata(&alias) {
        Ok(metadata) => {
            assert_eq!(
                (metadata.dev(), metadata.ino()),
                (original.dev(), original.ino()),
                "fresh case alias unexpectedly refers to another directory"
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("跳过大小写别名嵌套场景：临时文件系统区分大小写；验证独立根正常配对。");
            fs::create_dir(&alias).unwrap();
            let independent = fs::metadata(&alias).unwrap();
            assert_ne!(
                (independent.dev(), independent.ino()),
                (original.dev(), original.ino())
            );
            drop(FsVaultBroker::open(&fixture.vault, &alias).unwrap());
            return;
        }
        Err(error) => panic!("inspect case alias: {error}"),
    }

    assert!(matches!(
        FsVaultBroker::open(&fixture.vault, &alias.join("data")),
        Err(BrokerError::WriterBindingMismatch)
    ));
    assert_eq!(
        fs::symlink_metadata(fixture.vault.join("data"))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound,
        "alias preflight must reject before creating data inside canonical"
    );
}

#[test]
fn linked_roots_and_identity_nodes_cannot_gain_writer_authority() {
    for node in ["vault", "data", "control", "marker"] {
        let fixture = Fixture::initialized();
        let target = match node {
            "vault" => fixture.vault.clone(),
            "data" => fixture.data.clone(),
            "control" => fixture.vault.join(".workbench"),
            _ => fixture.marker(),
        };
        let moved = fixture.directory.path().join("moved");
        fs::rename(&target, &moved).unwrap();
        symlink(&moved, &target).unwrap();
        let before = inventory(fixture.directory.path());
        assert!(
            FsVaultBroker::open_existing(&fixture.vault, &fixture.data).is_err(),
            "accepted linked {node}"
        );
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}
