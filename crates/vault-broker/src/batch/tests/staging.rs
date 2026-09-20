//! 私有候选存储的正文隔离和完整性。
use super::{Fixture, path};
use crate::{ArtifactCandidate, ArtifactCommit, ArtifactStager, MemoryVaultBroker};
use std::fs;
fn artifacts() -> ArtifactCommit {
    ArtifactCommit {
        task_id: studiolo_core::TaskId::new(),
        attempt_id: studiolo_core::AttemptId::new(),
        fence: studiolo_core::Fence::new(1).unwrap(),
        audit_intent: "intent".into(),
        idempotency_key: "stage".into(),
        artifacts: vec![ArtifactCandidate {
            name: path("answer.md"),
            content: b"PRIVATE CANDIDATE BODY".to_vec(),
        }],
    }
}
#[test]
fn candidate_pointer_has_no_body_and_tampering_is_rejected() {
    let fixture = Fixture::new();
    let broker = fixture.open();
    let plan = artifacts();
    let reference = broker.stage_artifact_commit(&plan).unwrap();
    assert_eq!(
        broker.load_staged_artifact_commit(&reference).unwrap(),
        plan
    );
    let encoded = serde_json::to_string(&reference).unwrap();
    assert!(!encoded.contains("PRIVATE"));
    assert!(!encoded.contains("content"));
    fs::write(
        fixture
            .data
            .join(format!("artifact-candidate-{}.json", reference.id)),
        b"{}",
    )
    .unwrap();
    assert!(broker.load_staged_artifact_commit(&reference).is_err());
    let memory = MemoryVaultBroker::new();
    let reference = memory.stage_artifact_commit(&plan).unwrap();
    assert_eq!(
        memory.load_staged_artifact_commit(&reference).unwrap(),
        plan
    );
}

#[test]
fn pending_batch_can_roll_back_without_publishing_partial_state() {
    use crate::{BatchWriter, VaultReader};
    use std::sync::atomic::Ordering;
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let request = super::request("cancel-pending");
    let preview = broker.preview_batch(&request).unwrap();
    broker.writer.fault.store(42, Ordering::Release);
    assert!(broker.commit_batch(request, &preview).is_err());
    assert!(crate::CanonicalReadVault::open_existing(&fixture.vault).is_err());
    assert!(broker.rollback_batch("cancel-pending").unwrap().rolled_back);
    assert!(matches!(
        broker.read(&path("notes/A.md")),
        Err(crate::BrokerError::NotFound(_))
    ));
    assert!(matches!(
        broker.read(&path("notes/B.md")),
        Err(crate::BrokerError::NotFound(_))
    ));
}

#[test]
fn canonical_reader_already_open_observes_persistent_barrier() {
    use crate::{BatchWriter, VaultReader};
    use std::sync::atomic::Ordering;
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let reader = crate::CanonicalReadVault::open_existing(&fixture.vault).unwrap();
    let request = super::request("read-barrier");
    let preview = broker.preview_batch(&request).unwrap();
    broker.writer.fault.store(42, Ordering::Release);
    assert!(broker.commit_batch(request, &preview).is_err());
    assert!(matches!(
        reader.read_file(&path("notes/A.md"), 1024),
        Err(crate::BrokerError::MutationUnresolved)
    ));
    drop(broker);
    let broker = fixture.reopen();
    assert_eq!(
        reader.read_file(&path("notes/A.md"), 1024).unwrap().content,
        broker.read(&path("notes/A.md")).unwrap().content
    );
}
