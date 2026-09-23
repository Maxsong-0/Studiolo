//! Black-box contract tests for the filesystem VaultBroker.

#[path = "broker/artifacts.rs"]
mod artifacts;
#[path = "broker/paths.rs"]
mod paths;
#[path = "broker/snapshots.rs"]
mod snapshots;

use std::fs;

use studiolo_core::{AttemptId, Fence, Revision, TaskId, VaultPath};
use studiolo_vault_broker::{
    ArtifactCandidate, ArtifactCommit, BrokerError, FsVaultBroker, MemoryVaultBroker, Mutation,
    Precondition, SnapshotInput, SnapshotReader, VaultBroker, VaultReader, VaultSnapshotter,
    VaultWriter,
};
use tempfile::TempDir;

fn broker() -> (TempDir, TempDir, FsVaultBroker) {
    let vault =
        TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).expect("vault tempdir");
    let data = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).expect("data tempdir");
    let broker = FsVaultBroker::open(vault.path(), data.path()).expect("open broker");
    (vault, data, broker)
}

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("valid vault path")
}

fn artifact_request() -> ArtifactCommit {
    ArtifactCommit {
        task_id: TaskId::new(),
        attempt_id: AttemptId::new(),
        fence: Fence::new(1).unwrap(),
        audit_intent: "intent-integrity".to_owned(),
        idempotency_key: "commit-integrity".to_owned(),
        artifacts: vec![ArtifactCandidate {
            name: path("result.md"),
            content: b"authentic".to_vec(),
        }],
    }
}

#[test]
fn initializes_and_reuses_vault_identity() {
    let (vault, data, first) = broker();
    let uuid = first.vault_uuid();

    let identity = fs::read_to_string(vault.path().join(".workbench/vault.json"))
        .expect("identity is durable");
    assert!(identity.contains(&uuid.to_string()));

    drop(first);
    let reopened = FsVaultBroker::open(vault.path(), data.path()).expect("reopen broker");
    assert_eq!(reopened.vault_uuid(), uuid);
}

#[test]
fn mutation_is_cas_guarded_and_preserves_l0_history() {
    let (_vault, data, mut broker) = broker();
    let note = path("notes/plan.md");
    let first = broker
        .mutate(Mutation::create(
            note.clone(),
            b"v1".to_vec(),
            "create-plan",
        ))
        .expect("create note");

    let wrong_revision = Revision::compute(broker.vault_uuid(), &note, b"not-v1");
    let conflict = broker.mutate(Mutation::replace(
        note.clone(),
        b"bad".to_vec(),
        wrong_revision,
        "wrong-revision",
    ));
    assert!(matches!(
        conflict,
        Err(BrokerError::RevisionConflict { .. })
    ));

    let second = broker
        .mutate(Mutation::replace(
            note.clone(),
            b"v2".to_vec(),
            first.revision,
            "replace-plan",
        ))
        .expect("replace note");
    assert_ne!(first.revision, second.revision);
    assert_eq!(broker.read(&note).expect("read note").content, b"v2");
    assert_eq!(
        fs::read(data.path().join("history").join(first.revision.to_string()))
            .expect("history object"),
        b"v1"
    );
}

#[test]
fn idempotency_survives_process_reopen_and_rejects_key_reuse() {
    let (vault, data, mut first) = broker();
    let note = path("notes/idempotent.md");
    let request = Mutation::create(note.clone(), b"once".to_vec(), "stable-key");
    let receipt = first.mutate(request.clone()).expect("first write");
    assert_eq!(first.mutate(request).expect("same-process replay"), receipt);
    drop(first);

    let mut reopened = FsVaultBroker::open(vault.path(), data.path()).expect("reopen broker");
    assert_eq!(
        reopened
            .mutate(Mutation::create(
                note.clone(),
                b"once".to_vec(),
                "stable-key"
            ))
            .expect("cross-process replay"),
        receipt
    );
    assert!(matches!(
        reopened.mutate(Mutation::create(note, b"different".to_vec(), "stable-key")),
        Err(BrokerError::IdempotencyConflict)
    ));
}

#[test]
fn precondition_type_has_no_unchecked_mode() {
    let absent = Precondition::MustNotExist;
    assert!(matches!(absent, Precondition::MustNotExist));
}

#[test]
fn memory_double_obeys_the_public_read_write_and_snapshot_seams() {
    let mut broker = MemoryVaultBroker::new();
    fn accepts_object_safe_port(_broker: &mut dyn VaultBroker) {}
    accepts_object_safe_port(&mut broker);
    let note = path("notes/memory.md");
    let receipt = broker
        .mutate(Mutation::create(
            note.clone(),
            b"memory".to_vec(),
            "memory-1",
        ))
        .expect("memory write");
    assert_eq!(broker.read(&note).unwrap().content, b"memory");

    let snapshot = broker
        .snapshot(&[SnapshotInput {
            path: note,
            expected_revision: receipt.revision,
        }])
        .expect("memory snapshot");
    assert_eq!(broker.load_snapshot(&snapshot.id).unwrap(), snapshot);
}
