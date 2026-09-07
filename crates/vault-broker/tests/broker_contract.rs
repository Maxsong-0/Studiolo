//! Black-box contract tests for the filesystem VaultBroker.

use std::fs;

use studiolo_core::{AttemptId, Fence, Revision, TaskId, VaultPath};
use studiolo_vault_broker::{
    ArtifactCandidate, ArtifactCommit, BrokerError, FsVaultBroker, MemoryVaultBroker, Mutation,
    Precondition, SnapshotId, SnapshotInput, SnapshotReader, VaultBroker, VaultReader,
    VaultSnapshotter, VaultWriter,
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
fn artifact_commit_writes_manifest_last_and_replays_without_duplicates() {
    let (vault, _data, mut broker) = broker();
    let task_id = TaskId::new();
    let attempt_id = AttemptId::new();
    let request = ArtifactCommit {
        task_id,
        attempt_id,
        fence: Fence::new(1).expect("non-zero fence"),
        audit_intent: "audit-intent-1".to_owned(),
        idempotency_key: "commit-1".to_owned(),
        artifacts: vec![ArtifactCandidate {
            name: path("result.md"),
            content: b"done".to_vec(),
        }],
    };

    let first = broker
        .commit_attempt_artifacts(request.clone())
        .expect("commit artifacts");
    assert_eq!(
        first.manifest_path.as_str(),
        format!("sources/attempts/{task_id}/{attempt_id}/manifest.json")
    );
    assert!(vault.path().join(first.manifest_path.as_str()).is_file());
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(vault.path().join(first.manifest_path.as_str())).expect("manifest bytes"),
    )
    .expect("manifest JSON");
    assert_eq!(manifest["task_id"], task_id.to_string());
    assert_eq!(manifest["attempt_id"], attempt_id.to_string());
    assert_eq!(manifest["fence"], 1);
    assert_eq!(manifest["audit_intent"], "audit-intent-1");
    assert_eq!(
        manifest["artifacts"][0]["sha256"],
        first.artifacts[0].sha256
    );
    assert_eq!(
        fs::read(vault.path().join(first.artifacts[0].path.as_str())).expect("artifact"),
        b"done"
    );

    let replay = broker
        .commit_attempt_artifacts(request)
        .expect("idempotent replay");
    assert_eq!(replay, first);
    assert_eq!(first.artifacts.len(), 1);
}

#[test]
fn protected_paths_and_collisions_are_rejected() {
    let (_vault, _data, mut broker) = broker();
    assert!(matches!(
        broker.mutate(Mutation::create(
            path(".workbench/vault.json"),
            b"forged".to_vec(),
            "forge"
        )),
        Err(BrokerError::ProtectedPath(_))
    ));

    broker
        .mutate(Mutation::create(
            path("Notes/A.md"),
            b"one".to_vec(),
            "case-1",
        ))
        .expect("first spelling");
    assert!(matches!(
        broker.mutate(Mutation::create(
            path("notes/a.md"),
            b"two".to_vec(),
            "case-2"
        )),
        Err(BrokerError::PathCollision { .. })
    ));
}

#[test]
fn decomposed_entry_names_on_disk_collide_with_their_nfc_spelling() {
    let (vault, _data, mut broker) = broker();
    // 外部写者（HFS+ 时代或别的工具）留下 NFD 拼写：か + U+3099，与 NFC 的 が 是同一逻辑名。
    fs::create_dir_all(vault.path().join("notes")).unwrap();
    fs::write(vault.path().join("notes/か\u{3099}.md"), b"decomposed").unwrap();
    assert!(matches!(
        broker.mutate(Mutation::create(
            path("notes/が.md"),
            b"composed".to_vec(),
            "nfc-create"
        )),
        Err(BrokerError::PathCollision { .. })
    ));
    assert!(matches!(
        broker.read(&path("notes/が.md")),
        Err(BrokerError::PathCollision { .. })
    ));
}

#[cfg(unix)]
#[test]
fn symlink_hardlink_and_special_files_fail_closed() {
    use std::os::unix::{fs::symlink, net::UnixListener};

    let (vault, _data, mut broker) = broker();
    let outside =
        TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).expect("outside tempdir");
    fs::write(outside.path().join("secret.md"), b"outside").expect("outside file");
    symlink(outside.path(), vault.path().join("escape")).expect("create symlink");
    assert!(matches!(
        broker.read(&path("escape/secret.md")),
        Err(BrokerError::UnsafeNode { .. })
    ));

    let hard_receipt = broker
        .mutate(Mutation::create(path("hard.md"), b"one".to_vec(), "hard-1"))
        .expect("create hardlink source");
    fs::hard_link(
        vault.path().join("hard.md"),
        vault.path().join("hard-copy.md"),
    )
    .expect("create hardlink");
    assert!(matches!(
        broker.mutate(Mutation::replace(
            path("hard.md"),
            b"two".to_vec(),
            hard_receipt.revision,
            "hard-2"
        )),
        Err(BrokerError::UnsafeNode { .. })
    ));

    match UnixListener::bind(vault.path().join("socket")) {
        Ok(_socket) => assert!(matches!(
            broker.read(&path("socket")),
            Err(BrokerError::UnsafeNode { .. })
        )),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            // Some application sandboxes forbid creating Unix sockets. Linux CI
            // exercises this branch on an unrestricted temporary filesystem.
        }
        Err(error) => panic!("bind unix socket: {error}"),
    }
}

#[cfg(unix)]
#[test]
fn vault_identity_cannot_be_redirected_through_a_symlink() {
    use std::os::unix::fs::symlink;

    let vault = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let data = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let outside = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    fs::create_dir(vault.path().join(".workbench")).unwrap();
    fs::write(outside.path().join("vault.json"), b"{}").unwrap();
    symlink(
        outside.path().join("vault.json"),
        vault.path().join(".workbench/vault.json"),
    )
    .unwrap();

    assert!(matches!(
        FsVaultBroker::open(vault.path(), data.path()),
        Err(BrokerError::CorruptIdentity)
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

fn assert_replay_checks_artifact_content(broker: &mut impl VaultBroker) {
    let request = artifact_request();
    let receipt = broker.commit_attempt_artifacts(request.clone()).unwrap();
    let artifact = &receipt.artifacts[0];
    broker
        .mutate(Mutation::replace(
            artifact.path.clone(),
            b"tampered".to_vec(),
            artifact.revision,
            "tamper-artifact",
        ))
        .unwrap();
    assert!(matches!(
        broker.commit_attempt_artifacts(request),
        Err(BrokerError::ArtifactConflict(_))
    ));
}

#[test]
fn manifest_replay_verifies_current_artifact_content_in_both_adapters() {
    let (_vault, _data, mut disk) = broker();
    assert_replay_checks_artifact_content(&mut disk);
    assert_replay_checks_artifact_content(&mut MemoryVaultBroker::new());
}

#[test]
fn manifest_replay_rejects_tampered_metadata_and_missing_artifact_after_reopen() {
    for tampering in ["missing", "hash", "path", "fence", "format", "count"] {
        let (vault, data, mut broker) = broker();
        let request = artifact_request();
        let receipt = broker.commit_attempt_artifacts(request.clone()).unwrap();
        let manifest_path = vault.path().join(receipt.manifest_path.as_str());
        if tampering == "missing" {
            fs::remove_file(vault.path().join(receipt.artifacts[0].path.as_str())).unwrap();
        } else {
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
            match tampering {
                "hash" => manifest["artifacts"][0]["sha256"] = "forged".into(),
                "path" => manifest["artifacts"][0]["path"] = "unrelated.md".into(),
                "fence" => manifest["fence"] = 2.into(),
                "format" => manifest["format_version"] = 2.into(),
                "count" => manifest["artifacts"] = serde_json::json!([]),
                _ => unreachable!(),
            }
            fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        }
        drop(broker);
        let mut reopened = FsVaultBroker::open(vault.path(), data.path()).unwrap();
        assert!(
            reopened.commit_attempt_artifacts(request).is_err(),
            "accepted {tampering}"
        );
    }
}

#[test]
fn missing_orphan_replay_cannot_publish_a_manifest() {
    let (vault, _data, mut broker) = broker();
    let request = artifact_request();
    let artifact_path = path(&format!(
        "sources/attempts/{}/{}/result.md",
        request.task_id, request.attempt_id
    ));
    broker
        .mutate(Mutation::create(
            artifact_path.clone(),
            b"authentic".to_vec(),
            "artifact:commit-integrity:result.md",
        ))
        .unwrap();
    fs::remove_file(vault.path().join(artifact_path.as_str())).unwrap();
    assert!(broker.commit_attempt_artifacts(request.clone()).is_err());
    assert!(
        !vault
            .path()
            .join(format!(
                "sources/attempts/{}/{}/manifest.json",
                request.task_id, request.attempt_id
            ))
            .exists()
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
        assert!(serde_json::from_value::<SnapshotId>(serde_json::json!(invalid)).is_err());
    }
    assert!(
        serde_json::from_value::<SnapshotId>(serde_json::json!("0123456789abcdef".repeat(4)))
            .is_ok()
    );
}

#[test]
fn memory_double_rejects_aliasing_parent_names_and_protected_artifact_names() {
    let mut broker = MemoryVaultBroker::new();
    broker
        .mutate(Mutation::create(path("Notes/a.md"), b"a".to_vec(), "first"))
        .unwrap();
    assert!(matches!(
        broker.mutate(Mutation::create(
            path("notes/b.md"),
            b"b".to_vec(),
            "second"
        )),
        Err(BrokerError::PathCollision { .. })
    ));
    let mut commit = artifact_request();
    commit.artifacts[0].name = path(".workbench/private.md");
    assert!(matches!(
        broker.commit_attempt_artifacts(commit),
        Err(BrokerError::InvalidArtifact(_))
    ));
}
