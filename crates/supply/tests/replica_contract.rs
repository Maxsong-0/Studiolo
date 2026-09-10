//! 完整副本契约：范围、摘要、不可变批次、失败/过期、回执回退与故障关闭。
mod replica_support;

use replica_support::{FIRST, ReplicaFixture, SECOND, write};
use serde_json::json;
use std::fs;
use studiolo_supply::{
    EntryKind,
    replica::{ReplicaError, ReplicaRoot},
};

fn root(f: &ReplicaFixture) -> ReplicaRoot {
    ReplicaRoot::open(&f.root, &["project".into()], 300).unwrap()
}

#[test]
fn complete_snapshot_includes_empty_directories_and_verifies_binary_bytes() {
    let f = ReplicaFixture::new();
    let replica = root(&f).verify(f.observed + 1000, None).unwrap();
    assert_eq!(replica.snapshot.entries.len(), 5);
    assert_eq!(replica.snapshot.notes.len(), 2);
    assert!(replica.snapshot.diagnostics.is_empty());
    assert!(
        replica
            .snapshot
            .entries
            .iter()
            .any(|entry| entry.path.as_str() == "project/empty"
                && entry.kind == EntryKind::Directory)
    );
    let status = replica.freshness.describe(f.observed + 1000).unwrap();
    assert!(status.contains("状态 ready") && status.contains("age_secs=1"));
    assert!(
        replica
            .freshness
            .describe(f.observed + 301_000)
            .unwrap()
            .contains("状态 stale")
    );
}

#[test]
fn success_and_failure_keep_real_source_time_and_old_snapshot_immutable() {
    let f = ReplicaFixture::new();
    let root = root(&f);
    let old = root.verify(f.observed + 1000, None).unwrap();
    f.publish(SECOND, "newmarker", f.observed + 1000);
    let new = root
        .verify(f.observed + 2000, Some(&old.freshness))
        .unwrap();
    assert!(
        old.snapshot
            .notes
            .iter()
            .all(|note| !note.content.contains("newmarker"))
    );
    assert!(
        new.snapshot
            .notes
            .iter()
            .all(|note| !note.content.contains("oldmarker"))
    );
    let mut failed = f.state();
    failed["status"] = json!("failed");
    failed["last_error"] = json!("transfer_failed");
    failed["last_attempt_at_ms"] = json!(f.observed + 3000);
    f.write_state(&failed);
    let failed = root
        .verify(f.observed + 500_000, Some(&new.freshness))
        .unwrap();
    let status = failed.freshness.describe(f.observed + 500_000).unwrap();
    assert!(status.contains("sync_failed") && status.contains("stale=true"));
    assert_eq!(failed.freshness.receipt, new.freshness.receipt);
    assert_eq!(failed.snapshot, new.snapshot);
}

#[test]
fn files_attachments_missing_directories_and_extra_items_are_all_drift() {
    for action in 0..5 {
        let f = ReplicaFixture::new();
        match action {
            0 => write(&f.file("project/note.md"), b"# changed note"),
            1 => write(&f.file("project/image.bin"), &[2, 1, 255]),
            2 => fs::remove_dir(f.file("project/empty")).unwrap(),
            3 => write(&f.file("project/extra.md"), b"# unexpected"),
            _ => write(&f.file("project/.hidden"), b"hidden"),
        }
        assert!(
            matches!(
                root(&f).verify(f.observed + 1000, None),
                Err(ReplicaError::Drift(_))
            ),
            "action {action}"
        );
    }
}

#[test]
fn malformed_and_out_of_scope_manifests_fail_even_with_matching_raw_digest() {
    for action in 0..7 {
        let f = ReplicaFixture::new();
        f.rewrite_manifest(|value| match action {
            0 => value["entries"][0]["path"] = json!("outside/empty"),
            1 => value["entries"][0]["sha256"] = json!("a".repeat(64)),
            2 => value["entries"][4]["bytes"] = json!(16 * 1024 * 1024 + 1),
            3 => value["entries"].as_array_mut().unwrap().swap(0, 1),
            4 => value["allow"] = json!(["other"]),
            5 => value["extra"] = json!(true),
            _ => value["entries"][4]["path"] = json!("project/../escape.md"),
        });
        assert!(
            matches!(
                root(&f).verify(f.observed + 1000, None),
                Err(ReplicaError::Invalid(_))
            ),
            "action {action}"
        );
    }
}

#[test]
fn raw_manifest_tampering_and_hardlinks_fail_closed() {
    let f = ReplicaFixture::new();
    let path = f.release(FIRST).join("manifest.json");
    let mut bytes = fs::read(&path).unwrap();
    bytes.push(b' ');
    write(&path, &bytes);
    assert!(matches!(
        root(&f).verify(f.observed + 1000, None),
        Err(ReplicaError::Invalid("manifest_digest"))
    ));
    let f = ReplicaFixture::new();
    fs::hard_link(f.file("project/note.md"), f.root.join("linked.md")).unwrap();
    assert!(root(&f).verify(f.observed + 1000, None).is_err());
}

#[test]
fn malformed_future_or_reversed_receipts_fail_and_previous_receipts_cannot_regress() {
    for action in 0..7 {
        let f = ReplicaFixture::new();
        let mut state = f.state();
        match action {
            0 => state["receipt"]["generation"] = json!("../escape"),
            1 => state["receipt"]["source_observed_at_ms"] = json!(f.observed + 101),
            2 => state["last_attempt_at_ms"] = json!(f.observed + 100_000),
            3 => state["status"] = json!("failed"),
            4 => state["last_error"] = json!("transfer_failed"),
            5 => state["receipt"] = json!(null),
            _ => state["receipt"]["received_at_ms"] = json!(-1),
        }
        f.write_state(&state);
        assert!(
            root(&f).verify(f.observed + 1000, None).is_err(),
            "action {action}"
        );
    }
    let f = ReplicaFixture::new();
    let root = root(&f);
    let old_state = f.state();
    let first = root.verify(f.observed + 1000, None).unwrap();
    f.publish(SECOND, "newmarker", f.observed + 1000);
    let second = root
        .verify(f.observed + 2000, Some(&first.freshness))
        .unwrap();
    f.write_state(&old_state);
    assert!(matches!(
        root.verify(f.observed + 2000, Some(&second.freshness)),
        Err(ReplicaError::Invalid("receipt_regression"))
    ));
}

#[test]
fn explicit_null_protocol_fields_must_still_be_present() {
    let f = ReplicaFixture::new();
    let mut state = f.state();
    state.as_object_mut().unwrap().remove("last_error");
    f.write_state(&state);
    assert!(root(&f).verify(f.observed + 1000, None).is_err());
    let f = ReplicaFixture::new();
    f.rewrite_manifest(|manifest| {
        manifest["entries"][0]
            .as_object_mut()
            .unwrap()
            .remove("sha256");
    });
    assert!(root(&f).verify(f.observed + 1000, None).is_err());
}
