//! 兼容夹具只写已交付 M0 的磁盘格式，不模拟新协议的私有故障阶段。

use super::support::{Fixture, inventory, path};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
use studiolo_core::{Revision, VaultUuid};
use studiolo_vault_broker::{
    BrokerError, FsVaultBroker, Mutation, MutationReceipt, VaultReader, VaultWriter,
};

pub(super) struct Legacy {
    pub fixture: Fixture,
    pub request: Mutation,
    pub receipt: MutationReceipt,
    pub record: PathBuf,
}

impl Legacy {
    pub fn new(completed: bool) -> Self {
        let fixture = Fixture::empty();
        fs::create_dir_all(fixture.vault.join(".workbench")).unwrap();
        for directory in ["history", "idempotency", "snapshots"] {
            fs::create_dir_all(fixture.data.join(directory)).unwrap();
        }
        let vault_uuid = VaultUuid::new();
        fs::write(
            fixture.marker(),
            serde_json::to_vec(&serde_json::json!({
                "format_version": 1, "vault_uuid": vault_uuid,
            }))
            .unwrap(),
        )
        .unwrap();
        let request = Mutation::create(path("legacy.md"), b"legacy".to_vec(), "legacy-key");
        let revision = Revision::compute(vault_uuid, &request.path, &request.content);
        let receipt = MutationReceipt {
            path: request.path.clone(),
            revision,
            previous_revision: None,
        };
        let fingerprint = digest(&serde_json::to_vec(&request).unwrap());
        let record = fixture.data.join("idempotency").join(format!(
            "{}.json",
            digest(request.idempotency_key.as_bytes())
        ));
        fs::write(
            &record,
            serde_json::to_vec(&serde_json::json!({
                "fingerprint": fingerprint, "expected_revision": revision,
                "receipt": if completed { Some(&receipt) } else { None },
            }))
            .unwrap(),
        )
        .unwrap();
        // 未决夹具也放入相同字节：当前文件存在不构成原请求安装成功的证据。
        fs::write(fixture.vault.join("legacy.md"), &request.content).unwrap();
        Self {
            fixture,
            request,
            receipt,
            record,
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn existing_open_does_not_adopt_legacy_layout_implicitly() {
    let legacy = Legacy::new(true);
    let before = inventory(legacy.fixture.directory.path());
    assert!(FsVaultBroker::open_existing(&legacy.fixture.vault, &legacy.fixture.data).is_err());
    assert_eq!(inventory(legacy.fixture.directory.path()), before);
}

#[test]
fn explicit_legacy_adoption_preserves_completed_receipts_without_installing_them() {
    let legacy = Legacy::new(true);
    let old_record = fs::read(&legacy.record).unwrap();
    fs::write(legacy.fixture.vault.join("legacy.md"), b"external-current").unwrap();
    let mut broker = FsVaultBroker::open(&legacy.fixture.vault, &legacy.fixture.data).unwrap();
    assert_eq!(broker.write_status().unwrap().committed_through, 0);
    assert!(!broker.write_status().unwrap().unresolved);
    assert_eq!(
        broker.mutate(legacy.request.clone()).unwrap(),
        legacy.receipt
    );
    assert_eq!(
        broker.read(&path("legacy.md")).unwrap().content,
        b"external-current"
    );
    assert_eq!(broker.write_status().unwrap().committed_through, 0);
    assert_eq!(fs::read(&legacy.record).unwrap(), old_record);
    drop(broker);
    let mut reopened = legacy.fixture.open();
    assert_eq!(reopened.mutate(legacy.request).unwrap(), legacy.receipt);
    reopened
        .mutate(Mutation::create(path("new.md"), b"new".to_vec(), "new-key"))
        .unwrap();
    assert_eq!(reopened.write_status().unwrap().committed_through, 1);
    assert_eq!(fs::read(legacy.record).unwrap(), old_record);
}

#[test]
fn legacy_unknown_blocks_every_new_install_and_cannot_borrow_matching_current_bytes() {
    let legacy = Legacy::new(false);
    let original_record = fs::read(&legacy.record).unwrap();
    let mut broker = FsVaultBroker::open(&legacy.fixture.vault, &legacy.fixture.data).unwrap();
    for reopen in [false, true] {
        if reopen {
            drop(broker);
            broker = legacy.fixture.open();
        }
        let status = broker.write_status().unwrap();
        assert!(status.unresolved);
        assert_eq!(status.committed_through, 0);
        let before = inventory(&legacy.fixture.vault);
        assert!(matches!(
            broker.mutate(legacy.request.clone()),
            Err(BrokerError::MutationUnresolved)
        ));
        assert!(matches!(
            broker.mutate(Mutation::create(
                path("unrelated.md"),
                b"unrelated".to_vec(),
                "unrelated-key"
            )),
            Err(BrokerError::MutationUnresolved)
        ));
        assert!(matches!(
            broker.commit_attempt_artifacts(super::replacement::artifacts()),
            Err(BrokerError::MutationUnresolved)
        ));
        assert_eq!(inventory(&legacy.fixture.vault), before);
        assert_eq!(broker.read(&path("legacy.md")).unwrap().content, b"legacy");
        assert_eq!(fs::read(&legacy.record).unwrap(), original_record);
    }
    drop(broker);
    let alternate = legacy.fixture.data.with_file_name("alternate-data");
    fs::create_dir(&alternate).unwrap();
    assert!(matches!(
        FsVaultBroker::open(&legacy.fixture.vault, &alternate),
        Err(BrokerError::WriterBindingMismatch)
    ));
    assert!(inventory(&alternate).is_empty());
}

#[test]
fn malformed_and_linked_legacy_records_prevent_adoption_before_creating_new_state() {
    for damage in ["invalid-json", "symlink", "hardlink"] {
        let legacy = Legacy::new(true);
        let outside = legacy.fixture.directory.path().join("preserved-legacy");
        match damage {
            "invalid-json" => fs::write(&legacy.record, b"invalid").unwrap(),
            _ => {
                fs::rename(&legacy.record, &outside).unwrap();
                if damage == "symlink" {
                    std::os::unix::fs::symlink(&outside, &legacy.record).unwrap();
                } else {
                    fs::hard_link(&outside, &legacy.record).unwrap();
                }
            }
        }
        let before = inventory(legacy.fixture.directory.path());
        assert!(
            FsVaultBroker::open(&legacy.fixture.vault, &legacy.fixture.data).is_err(),
            "accepted {damage}"
        );
        assert_eq!(inventory(legacy.fixture.directory.path()), before);
    }
}
