//! 真实封存派生确定的非正文最终记录；远端提交本身由另一个适配器证明。
use super::staging::fixture::Fixture;
use std::num::NonZeroU64;
use studiolo_backup::{
    checkpoint::staging::SourceCatalog,
    worker::{
        final_manifest::{FinalManifestDraft, FinalManifestError, FinalManifestReceipt},
        restic::SnapshotRef,
        state::{JobKey, RemoteVerification},
    },
};
use studiolo_core::{
    BackupScope,
    checkpoint::{CheckpointId, InstantMillis},
};

fn key(id: &str) -> JobKey {
    JobKey {
        sequence: NonZeroU64::new(1).unwrap(),
        checkpoint_id: CheckpointId::parse(id).unwrap(),
    }
}
fn proof(at: i64) -> RemoteVerification {
    RemoteVerification {
        objects_complete: true,
        checksum_verified: true,
        paired_checkpoint_reference: true,
        verified_at: InstantMillis::from_millis(at),
    }
}
fn draft(f: &Fixture) -> FinalManifestDraft {
    let area = f.area("final");
    let object = f.object(
        &area,
        "canonical/note",
        b"fictional body must not enter final marker",
        BackupScope::Full,
    );
    let seal = area
        .seal(&SourceCatalog::new([object]).unwrap(), f.inputs())
        .unwrap();
    FinalManifestDraft::new(
        key("final"),
        &seal,
        SnapshotRef::parse("ab".repeat(32)).unwrap(),
        proof(2000),
    )
    .unwrap()
}
#[test]
fn final_marker_is_canonical_bounded_and_binds_the_original_data_snapshot() {
    let f = Fixture::new();
    let draft = draft(&f);
    let bytes = draft.encode().unwrap();
    assert_eq!(FinalManifestDraft::decode(&bytes).unwrap(), draft);
    assert_eq!(draft.sha256().unwrap().len(), 64);
    assert!(!String::from_utf8_lossy(&bytes).contains("fictional body"));
    assert_eq!(draft.key(), &key("final"));
    assert_eq!(draft.source().checkpoint_id, key("final").checkpoint_id);
    assert_eq!(draft.data_snapshot().as_str(), "ab".repeat(32));
    assert!(FinalManifestDraft::decode(&serde_json::to_vec_pretty(&draft).unwrap()).is_err());
    for (field, value) in [
        ("schema", serde_json::json!(2)),
        ("key", serde_json::json!(key("other"))),
        ("manifest_sha256", serde_json::json!("cd".repeat(32))),
        ("source_root", serde_json::json!("/root/../other")),
        ("data_snapshot", serde_json::json!("abcdef12")),
        ("data_verified_at", serde_json::json!(800)),
        ("data_verified_at", serde_json::json!(9000)),
        ("extra", serde_json::json!(true)),
    ] {
        let mut changed = serde_json::to_value(&draft).unwrap();
        changed[field] = value;
        assert!(
            serde_json::from_value::<FinalManifestDraft>(changed).is_err(),
            "{field}"
        );
    }
}
#[test]
fn wrong_job_unverified_data_short_id_and_oversized_manifest_never_form_a_draft() {
    let f = Fixture::new();
    let seal = f
        .area("final")
        .seal(&SourceCatalog::new([]).unwrap(), f.inputs())
        .unwrap();
    let snapshot = SnapshotRef::parse("ab".repeat(32)).unwrap();
    assert!(FinalManifestDraft::new(key("other"), &seal, snapshot.clone(), proof(2000)).is_err());
    assert!(
        FinalManifestDraft::new(
            key("final"),
            &seal,
            SnapshotRef::parse("abcdef12").unwrap(),
            proof(2000)
        )
        .is_err()
    );
    let mut invalid = proof(2000);
    invalid.checksum_verified = false;
    assert!(FinalManifestDraft::new(key("final"), &seal, snapshot.clone(), invalid).is_err());
    let mut inputs = f.inputs();
    inputs.manifest.application_packages[0].name = "x".repeat(64 * 1024);
    let large = f
        .area("large")
        .seal(&SourceCatalog::new([]).unwrap(), inputs)
        .unwrap();
    assert_eq!(
        FinalManifestDraft::new(key("large"), &large, snapshot, proof(2000)).err(),
        Some(FinalManifestError::TooLarge)
    );
}
#[test]
fn final_receipt_requires_another_snapshot_and_retains_original_record_and_times() {
    let f = Fixture::new();
    let draft = draft(&f);
    let final_snapshot = SnapshotRef::parse("cd".repeat(32)).unwrap();
    let receipt = FinalManifestReceipt::new(
        draft.clone(),
        final_snapshot.clone(),
        InstantMillis::from_millis(3000),
    )
    .unwrap();
    assert_eq!(receipt.draft(), &draft);
    assert_eq!(receipt.final_snapshot(), &final_snapshot);
    assert_eq!(
        serde_json::from_slice::<FinalManifestReceipt>(&serde_json::to_vec(&receipt).unwrap())
            .unwrap(),
        receipt
    );
    assert!(
        FinalManifestReceipt::new(
            draft.clone(),
            draft.data_snapshot().clone(),
            InstantMillis::from_millis(3000)
        )
        .is_err()
    );
    assert!(
        FinalManifestReceipt::new(
            draft.clone(),
            final_snapshot,
            InstantMillis::from_millis(1999)
        )
        .is_err()
    );
    for field in ["record_sha256", "final_snapshot"] {
        let mut changed = serde_json::to_value(&receipt).unwrap();
        changed[field] = serde_json::json!("abcd1234");
        assert!(serde_json::from_value::<FinalManifestReceipt>(changed).is_err());
    }
}
