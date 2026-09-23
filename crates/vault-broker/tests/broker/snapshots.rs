//! snapshot 按 revision 取材、身份稳定，持久记录与请求 ID、Vault、路径和内容绑定。

use std::fs;

use studiolo_vault_broker::{
    BrokerError, Mutation, SnapshotId, SnapshotInput, SnapshotReader, VaultSnapshotter, VaultWriter,
};

use super::{broker, path};

#[test]
fn snapshot_is_revision_checked_and_has_a_stable_identity() {
    let (_vault, _data, mut broker) = broker();
    let note = path("notes/source.md");
    let written = broker
        .mutate(Mutation::create(note.clone(), b"source".to_vec(), "source"))
        .expect("create source");

    let request = vec![SnapshotInput {
        path: note,
        expected_revision: written.revision,
    }];
    let first = broker.snapshot(&request).expect("snapshot");
    let second = broker.snapshot(&request).expect("repeat snapshot");
    assert_eq!(first.id, second.id);
    assert_eq!(first.files[0].content, b"source");
    assert_eq!(
        broker.load_snapshot(&first.id).expect("persisted snapshot"),
        first
    );
}

#[test]
fn stored_snapshot_is_bound_to_requested_id_vault_paths_and_content() {
    for tampering in ["content", "vault", "id", "path", "duplicate", "empty"] {
        let (_vault, data, mut broker) = broker();
        let note = path("notes/snapshot.md");
        let written = broker
            .mutate(Mutation::create(
                note.clone(),
                b"original".to_vec(),
                "snapshot-input",
            ))
            .unwrap();
        let snapshot = broker
            .snapshot(&[SnapshotInput {
                path: note,
                expected_revision: written.revision,
            }])
            .unwrap();
        let storage = data
            .path()
            .join("snapshots")
            .join(format!("{}.json", snapshot.id));
        let mut record = serde_json::to_value(&snapshot).unwrap();
        match tampering {
            "content" => record["files"][0]["content"] = serde_json::json!([1, 2, 3]),
            "vault" => {
                record["vault_uuid"] =
                    serde_json::to_value(studiolo_core::VaultUuid::new()).unwrap()
            }
            "id" => record["id"] = "0".repeat(64).into(),
            "path" => record["files"][0]["path"] = "different.md".into(),
            "duplicate" => {
                record["files"] = serde_json::json!([record["files"][0], record["files"][0]])
            }
            "empty" => record["files"] = serde_json::json!([]),
            _ => unreachable!(),
        }
        fs::write(storage, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(
            matches!(
                broker.load_snapshot(&snapshot.id),
                Err(BrokerError::CorruptSnapshot)
            ),
            "accepted {tampering}"
        );
    }
}

#[test]
fn snapshot_ids_cannot_deserialize_paths_or_noncanonical_digests() {
    for invalid in [
        "../history/secret",
        "",
        "abc",
        &"A".repeat(64),
        &"g".repeat(64),
    ] {
        let error = serde_json::from_value::<SnapshotId>(serde_json::json!(invalid))
            .expect_err("非规范 snapshot id 必须被拒绝");
        // 反序列化层的数据错误，由 SnapshotId 自身的校验给出，而不是 JSON 语法错误。
        assert_eq!(
            error.classify(),
            serde_json::error::Category::Data,
            "{invalid}"
        );
        assert_eq!(
            error.to_string(),
            "snapshot id must be 64 lowercase hexadecimal characters",
            "{invalid}"
        );
    }
    assert!(
        serde_json::from_value::<SnapshotId>(serde_json::json!("0123456789abcdef".repeat(4)))
            .is_ok()
    );
}
