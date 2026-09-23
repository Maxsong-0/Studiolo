use super::support::{Fixture, inventory, linked_record, path};
use std::{fs, io::ErrorKind, os::unix::fs::symlink, path::PathBuf};
use studiolo_vault_broker::{BrokerError, FsVaultBroker, Mutation, VaultWriter};

fn records(fixture: &Fixture) -> [PathBuf; 3] {
    [
        fixture.vault.join(".workbench/broker-binding.json"),
        fixture.data.join("broker-binding.json"),
        fixture.data.join("writer-state.json"),
    ]
}

/// 每种记录损坏对应的精确拒绝；链接的拒绝方式随平台的 rooted 打开实现而定。
fn rejected(damage: &str, error: &BrokerError) -> bool {
    match damage {
        "invalid-json" | "unknown-fields" => matches!(error, BrokerError::CorruptWriterState),
        "oversized" => matches!(error, BrokerError::WriterLimitExceeded),
        "symlink" => linked_record(error),
        "hardlink" => matches!(
            error,
            BrokerError::UnsafeNode {
                reason: "hard-linked or unlinked file",
                ..
            }
        ),
        "directory" => matches!(
            error,
            BrokerError::UnsafeNode {
                reason: "not a regular file",
                ..
            }
        ),
        _ => unreachable!("unknown damage {damage}"),
    }
}

#[test]
fn missing_one_side_of_binding_or_writer_state_is_not_silently_repaired() {
    for index in 0..3 {
        let fixture = Fixture::initialized();
        fs::remove_file(&records(&fixture)[index]).unwrap();
        let before = inventory(fixture.directory.path());
        // 两侧绑定缺一即配对不符；绑定完好而写者状态缺失属于持久状态损坏。
        let missing_state = index == 2;
        for result in [
            FsVaultBroker::open_existing(&fixture.vault, &fixture.data),
            FsVaultBroker::open(&fixture.vault, &fixture.data),
        ] {
            let expected = if missing_state {
                matches!(result, Err(BrokerError::CorruptWriterState))
            } else {
                matches!(result, Err(BrokerError::WriterBindingMismatch))
            };
            assert!(expected, "record {index}: {:?}", result.err());
        }
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}

#[test]
fn existing_open_requires_the_prepared_stage_directory_without_creating_it() {
    let fixture = Fixture::initialized();
    fs::remove_dir(fixture.vault.join(".workbench/staging")).unwrap();
    let before = inventory(fixture.directory.path());
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
            let result = FsVaultBroker::open_existing(&fixture.vault, &fixture.data);
            assert!(
                matches!(&result, Err(error) if rejected(damage, error)),
                "accepted {damage}, record {index}: {:?}",
                result.err()
            );
            let result = FsVaultBroker::open(&fixture.vault, &fixture.data);
            assert!(
                matches!(&result, Err(error) if rejected(damage, error)),
                "repaired {damage}, record {index}: {:?}",
                result.err()
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
    assert!(matches!(
        broker.write_status(),
        Err(BrokerError::CorruptWriterState)
    ));
    assert!(matches!(
        broker.mutate(Mutation::create(
            path("forbidden.md"),
            b"forbidden".to_vec(),
            "after-corruption"
        )),
        Err(BrokerError::CorruptWriterState)
    ));
    assert!(matches!(
        broker.commit_attempt_artifacts(super::replacement::artifacts()),
        Err(BrokerError::CorruptWriterState)
    ));
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
        let result = broker.mutate(request.clone());
        assert!(
            matches!(&result, Err(error) if rejected(damage, error)),
            "replayed damaged {damage} record: {:?}",
            result.err()
        );
        assert_eq!(
            fs::read(fixture.vault.join("note.md")).unwrap(),
            b"original"
        );
        drop(broker);
        let before = inventory(fixture.directory.path());
        // 可以在打开时拒绝，或在按 key 读取时拒绝；不要求重开遍历全部历史回执。
        // 分支自证：两支都算通过，日志写清本次走了哪一支；两支读到的是同一条损坏回执。
        let result = match FsVaultBroker::open_existing(&fixture.vault, &fixture.data) {
            Ok(mut reopened) => {
                eprintln!("损坏回执重开·分支自证：{damage} → 重开成功，按 key 重放时拒绝");
                reopened.mutate(request).map(drop)
            }
            Err(error) => {
                eprintln!("损坏回执重开·分支自证：{damage} → 重开即拒绝：{error:?}");
                Err(error)
            }
        };
        assert!(
            matches!(&result, Err(error) if rejected(damage, error)),
            "replayed reopened {damage} record: {result:?}"
        );
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}
