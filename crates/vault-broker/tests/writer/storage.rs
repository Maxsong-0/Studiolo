use super::support::{Fixture, inventory, path};
use std::{fs, os::unix::fs::symlink, path::PathBuf};
use studiolo_vault_broker::{FsVaultBroker, Mutation, VaultWriter};

fn records(fixture: &Fixture) -> [PathBuf; 3] {
    [
        fixture.vault.join(".workbench/broker-binding.json"),
        fixture.data.join("broker-binding.json"),
        fixture.data.join("writer-state.json"),
    ]
}

#[test]
fn missing_one_side_of_binding_or_writer_state_is_not_silently_repaired() {
    for index in 0..3 {
        let fixture = Fixture::initialized();
        fs::remove_file(&records(&fixture)[index]).unwrap();
        let before = inventory(fixture.directory.path());
        assert!(FsVaultBroker::open_existing(&fixture.vault, &fixture.data).is_err());
        assert!(FsVaultBroker::open(&fixture.vault, &fixture.data).is_err());
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}

#[test]
fn existing_open_requires_the_prepared_stage_directory_without_creating_it() {
    let fixture = Fixture::initialized();
    fs::remove_dir(fixture.vault.join(".workbench/staging")).unwrap();
    let before = inventory(fixture.directory.path());
    assert!(FsVaultBroker::open_existing(&fixture.vault, &fixture.data).is_err());
    assert_eq!(inventory(fixture.directory.path()), before);
}

#[test]
fn malformed_or_linked_writer_records_are_refused_without_repair() {
    for index in 0..3 {
        for damage in [
            "invalid-json",
            "unknown-fields",
            "symlink",
            "hardlink",
            "directory",
        ] {
            let fixture = Fixture::initialized();
            let record = records(&fixture)[index].clone();
            let outside = fixture.directory.path().join("preserved-record");
            let original = fs::read(&record).unwrap();
            match damage {
                "invalid-json" => fs::write(&record, b"{").unwrap(),
                "unknown-fields" => {
                    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
                    value["unexpected_field"] = true.into();
                    fs::write(&record, serde_json::to_vec(&value).unwrap()).unwrap();
                }
                "directory" => {
                    fs::remove_file(&record).unwrap();
                    fs::create_dir(&record).unwrap();
                }
                _ => {
                    fs::rename(&record, &outside).unwrap();
                    if damage == "symlink" {
                        symlink(&outside, &record).unwrap();
                    } else {
                        fs::hard_link(&outside, &record).unwrap();
                    }
                }
            }
            let before = inventory(fixture.directory.path());
            assert!(
                FsVaultBroker::open_existing(&fixture.vault, &fixture.data).is_err(),
                "accepted {damage}, record {index}"
            );
            assert!(
                FsVaultBroker::open(&fixture.vault, &fixture.data).is_err(),
                "repaired {damage}, record {index}"
            );
            assert_eq!(inventory(fixture.directory.path()), before);
        }
    }
}

#[test]
fn live_writer_rechecks_durable_state_before_another_installation() {
    let fixture = Fixture::initialized();
    let mut broker = fixture.open();
    fs::write(fixture.data.join("writer-state.json"), b"invalid").unwrap();
    let before = inventory(fixture.directory.path());
    assert!(broker.write_status().is_err());
    assert!(
        broker
            .mutate(Mutation::create(
                path("forbidden.md"),
                b"forbidden".to_vec(),
                "after-corruption"
            ))
            .is_err()
    );
    assert!(
        broker
            .commit_attempt_artifacts(super::replacement::artifacts())
            .is_err()
    );
    assert_eq!(inventory(fixture.directory.path()), before);
}

#[test]
fn completed_replay_records_are_checked_again_in_process_and_after_reopen() {
    for damage in ["invalid-json", "oversized", "symlink", "hardlink"] {
        let fixture = Fixture::initialized();
        let mut broker = fixture.open();
        let request = Mutation::create(path("note.md"), b"original".to_vec(), "record-check");
        broker.mutate(request.clone()).unwrap();
        let mut entries = fs::read_dir(fixture.data.join("idempotency")).unwrap();
        let record = entries.next().unwrap().unwrap().path();
        assert!(entries.next().is_none());
        let outside = fixture.directory.path().join("original-record");
        match damage {
            "invalid-json" => fs::write(&record, b"invalid").unwrap(),
            "oversized" => fs::write(&record, vec![b' '; 64 * 1024 + 1]).unwrap(),
            _ => {
                fs::rename(&record, &outside).unwrap();
                if damage == "symlink" {
                    symlink(&outside, &record).unwrap();
                } else {
                    fs::hard_link(&outside, &record).unwrap();
                }
            }
        }
        assert!(
            broker.mutate(request.clone()).is_err(),
            "replayed damaged {damage} record"
        );
        assert_eq!(
            fs::read(fixture.vault.join("note.md")).unwrap(),
            b"original"
        );
        drop(broker);
        let before = inventory(fixture.directory.path());
        // 可以在打开时拒绝，或在按 key 读取时拒绝；不要求重开遍历全部历史回执。
        if let Ok(mut reopened) = FsVaultBroker::open_existing(&fixture.vault, &fixture.data) {
            assert!(
                reopened.mutate(request).is_err(),
                "replayed reopened {damage} record"
            );
        }
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}
