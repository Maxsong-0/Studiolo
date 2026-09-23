use super::support::{Fixture, inventory};
use std::{fs, io::ErrorKind, os::unix::fs::symlink};
use studiolo_vault_broker::{BrokerError, FsVaultBroker, VaultReader};

#[test]
fn explicit_initialization_requires_two_existing_empty_roots() {
    let fixture = Fixture::empty();
    // 显式初始化不代建任何一侧根目录：缺哪一侧就在打开该侧时失败。
    assert!(matches!(
        FsVaultBroker::initialize_empty(&fixture.vault, &fixture.data),
        Err(BrokerError::Io {
            operation: "open writer directory",
            source,
        }) if source.kind() == ErrorKind::NotFound
    ));
    assert!(!fixture.vault.exists());
    assert!(!fixture.data.exists());
    fs::create_dir(&fixture.vault).unwrap();
    assert!(matches!(
        FsVaultBroker::initialize_empty(&fixture.vault, &fixture.data),
        Err(BrokerError::Io {
            operation: "open writer directory",
            source,
        }) if source.kind() == ErrorKind::NotFound
    ));
    assert!(inventory(&fixture.vault).is_empty());
    assert!(!fixture.data.exists());
    fs::create_dir(&fixture.data).unwrap();
    let writer = FsVaultBroker::initialize_empty(&fixture.vault, &fixture.data).unwrap();
    let uuid = writer.vault_uuid();
    drop(writer);
    assert_eq!(fixture.open().vault_uuid(), uuid);
    let before = inventory(fixture.directory.path());
    // 已初始化的根不再为空，重复初始化按配对不符拒绝。
    assert!(matches!(
        FsVaultBroker::initialize_empty(&fixture.vault, &fixture.data),
        Err(BrokerError::WriterBindingMismatch)
    ));
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
            assert!(
                matches!(
                    FsVaultBroker::initialize_empty(&fixture.vault, &fixture.data),
                    Err(BrokerError::WriterBindingMismatch)
                ),
                "accepted nonempty {node} with {name}"
            );
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
    // 链接的 data 根在逐级 NOFOLLOW 检查祖先时即失败；重叠的两根按配对不符拒绝。
    assert!(matches!(
        FsVaultBroker::initialize_empty(&fixture.vault, &link),
        Err(BrokerError::Io {
            operation: "inspect writer ancestry",
            ..
        })
    ));
    for data in [fixture.vault.clone(), fixture.vault.join("nested")] {
        assert!(
            matches!(
                FsVaultBroker::initialize_empty(&fixture.vault, &data),
                Err(BrokerError::WriterBindingMismatch)
            ),
            "{}",
            data.display()
        );
    }
    assert_eq!(inventory(fixture.directory.path()), before);
}

#[test]
fn existing_open_creates_neither_roots_nor_missing_identity() {
    let fixture = Fixture::empty();
    assert!(matches!(
        FsVaultBroker::open_existing(&fixture.vault, &fixture.data),
        Err(BrokerError::Io {
            operation: "open writer directory",
            source,
        }) if source.kind() == ErrorKind::NotFound
    ));
    assert!(!fixture.vault.exists());
    assert!(!fixture.data.exists());
    fs::create_dir(&fixture.vault).unwrap();
    fs::create_dir(&fixture.data).unwrap();
    let before = inventory(fixture.directory.path());
    // 空根缺少控制目录；普通重开不补建布局。
    assert!(matches!(
        FsVaultBroker::open_existing(&fixture.vault, &fixture.data),
        Err(BrokerError::Io {
            operation: "inspect writer layout",
            source,
        }) if source.kind() == ErrorKind::NotFound
    ));
    assert_eq!(inventory(fixture.directory.path()), before);
}

#[test]
fn existing_open_does_not_repair_removed_data_layout() {
    for directory in ["history", "idempotency", "snapshots"] {
        let fixture = Fixture::initialized();
        fs::remove_dir(fixture.data.join(directory)).unwrap();
        let before = inventory(fixture.directory.path());
        assert!(
            matches!(
                FsVaultBroker::open_existing(&fixture.vault, &fixture.data),
                Err(BrokerError::Io {
                    operation: "inspect writer layout",
                    source,
                }) if source.kind() == ErrorKind::NotFound
            ),
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
            matches!(
                FsVaultBroker::open(&vault, &data),
                Err(BrokerError::WriterBindingMismatch)
            ),
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
    // 分支自证：卷是否区分大小写决定走哪一支；打印判据原始输入（别名的查询结果）。
    let probe = fs::metadata(&alias);
    eprintln!(
        "大小写别名·分支自证：VAULT 查询 = {:?}（Ok 即别名指回原目录，验证嵌套 data 根被拒绝）",
        probe
            .as_ref()
            .map(|metadata| (metadata.dev(), metadata.ino()))
            .map_err(|error| error.kind())
    );
    match probe {
        Ok(metadata) => {
            assert_eq!(
                (metadata.dev(), metadata.ino()),
                (original.dev(), original.ino()),
                "fresh case alias unexpectedly refers to another directory"
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("跳过：临时文件系统区分大小写，无法构造指回原目录的别名；改验独立根正常配对");
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
        let result = FsVaultBroker::open_existing(&fixture.vault, &fixture.data);
        // 根被换成链接时在逐级 NOFOLLOW 打开处失败；控制目录与 marker 被换成链接时按
        // 配对不符与身份损坏拒绝。
        let rejected = match node {
            "vault" => matches!(
                &result,
                Err(BrokerError::Io {
                    operation: "open writer directory",
                    ..
                })
            ),
            "data" => matches!(
                &result,
                Err(BrokerError::Io {
                    operation: "inspect writer ancestry",
                    ..
                })
            ),
            "control" => matches!(&result, Err(BrokerError::WriterBindingMismatch)),
            _ => matches!(&result, Err(BrokerError::CorruptIdentity)),
        };
        assert!(rejected, "accepted linked {node}: {:?}", result.err());
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}
