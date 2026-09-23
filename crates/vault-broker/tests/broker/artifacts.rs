//! 产物提交先写产物、最后写清单；重放逐项复核清单与当前产物内容。

use std::fs;

use studiolo_core::{AttemptId, Fence, TaskId};
use studiolo_vault_broker::{
    ArtifactCandidate, ArtifactCommit, BrokerError, FsVaultBroker, MemoryVaultBroker, Mutation,
    VaultBroker, VaultWriter,
};

use super::{artifact_request, broker, path};

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
        let artifact = &receipt.artifacts[0].path;
        let result = reopened.commit_attempt_artifacts(request);
        // 产物缺失按读取失败上报；产物记录被改指向产物路径；清单头部被改指向清单本身。
        let rejected = match tampering {
            "missing" => matches!(&result, Err(BrokerError::NotFound(at)) if at == artifact),
            "hash" | "path" => {
                matches!(&result, Err(BrokerError::ArtifactConflict(at)) if at == artifact)
            }
            _ => matches!(
                &result,
                Err(BrokerError::ArtifactConflict(at)) if *at == receipt.manifest_path
            ),
        };
        assert!(rejected, "accepted {tampering}: {:?}", result.err());
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
    assert!(matches!(
        broker.commit_attempt_artifacts(request.clone()),
        Err(BrokerError::NotFound(missing)) if missing == artifact_path
    ));
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
