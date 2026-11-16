//! 清单第 2 版保存显式来源；旧清单缺字段时不能升成完整范围，过期正文不再写入。
use super::{artifact_request, broker};
use std::fs;
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    SourcePolicyRef,
};
use studiolo_core::{TrustedExportClass, VaultUuid};
use studiolo_vault_broker::{
    ArtifactStager, BrokerError, FsVaultBroker, MemoryVaultBroker, VaultReader, VaultWriter,
    read_trusted_export,
};

fn labels() -> DataLabels {
    DataLabels::new(
        Classification::Restricted,
        BackupScope::Full,
        DesktopSync::Deny,
        ModelEgress::Deny,
    )
    .unwrap()
}

fn provenance(retain: i64, approval: &str) -> ArtifactProvenance {
    ArtifactProvenance::new(
        SourcePolicyRef::new("fixture-source", "a".repeat(64)).unwrap(),
        labels(),
        retain,
        approval,
    )
    .unwrap()
}

#[test]
fn exact_replay_keeps_policy_and_expiry() {
    let (vault, _data, mut broker) = broker();
    let request = artifact_request();
    let receipt = broker.commit_attempt_artifacts(request.clone()).unwrap();
    let path = vault.path().join(receipt.manifest_path.as_str());
    let before = fs::read(&path).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(manifest["format_version"], 2);
    assert_eq!(
        manifest["provenance"]["retain_until_unix_seconds"],
        4_000_000_000_i64
    );
    assert_eq!(
        manifest["provenance"]["approval_sha256"],
        request.provenance.approval_sha256()
    );
    assert_eq!(
        broker.commit_attempt_artifacts(request.clone()).unwrap(),
        receipt
    );
    let mut changed = request;
    changed.provenance = provenance(3_000_000_000, &"c".repeat(64));
    assert!(matches!(
        broker.commit_attempt_artifacts(changed),
        Err(BrokerError::IdempotencyConflict)
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn old_manifest_missing_provenance_is_not_upgraded_to_full() {
    let (vault, data, mut broker) = broker();
    let request = artifact_request();
    let receipt = broker.commit_attempt_artifacts(request.clone()).unwrap();
    let path = vault.path().join(receipt.manifest_path.as_str());
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["format_version"] = 1.into();
    manifest.as_object_mut().unwrap().remove("provenance");
    let rewritten = serde_json::to_vec(&manifest).unwrap();
    fs::write(&path, &rewritten).unwrap();
    drop(broker);
    let mut reopened = FsVaultBroker::open(vault.path(), data.path()).unwrap();
    let error = reopened.commit_attempt_artifacts(request).unwrap_err();
    assert!(
        matches!(error, BrokerError::InvalidArtifact(ref message) if message.contains("不能升为完整范围")),
        "{error:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), rewritten);
    let after: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(after.get("provenance").is_none());
    assert_eq!(after["format_version"], 1);
}

#[test]
fn new_staging_and_body_writes_reject_expired_material() {
    let mut request = artifact_request();
    request.provenance = provenance(1, &"d".repeat(64));
    let (vault, _data, mut disk) = broker();
    let error = disk.commit_attempt_artifacts(request.clone()).unwrap_err();
    assert!(
        matches!(error, BrokerError::InvalidArtifact(ref message) if message.contains("留存期限已过")),
        "{error:?}"
    );
    assert!(
        !vault
            .path()
            .join(format!(
                "sources/attempts/{}/{}/manifest.json",
                request.task_id, request.attempt_id
            ))
            .exists()
    );
    let memory = MemoryVaultBroker::new();
    assert!(matches!(
        memory.stage_artifact_commit(&request),
        Err(BrokerError::InvalidArtifact(ref message)) if message.contains("留存期限已过")
    ));
    assert!(matches!(
        disk.stage_artifact_commit(&request),
        Err(BrokerError::InvalidArtifact(_))
    ));
}

#[test]
fn successful_stage_can_be_discarded_without_deleting_a_different_candidate() {
    let (_vault, data, broker) = broker();
    let request = artifact_request();
    let pointer = broker.stage_artifact_commit(&request).unwrap();
    let file = data
        .path()
        .join(format!("artifact-candidate-{}.json", pointer.id));
    assert!(file.is_file());
    broker.discard_staged_artifact_commit(&pointer).unwrap();
    assert!(!file.exists());
    assert!(broker.load_staged_artifact_commit(&pointer).is_err());
    broker.discard_staged_artifact_commit(&pointer).unwrap();
}

/// 真实 Broker 写出的暂存正文装着产物全文：导出扫描要把它认成来源资料、带它自己的原批准，
/// 不能按顶层目录归成没有期限的写者状态。读不出原批准的（换了 Vault 身份、升级前的旧暂存）整批拒绝。
#[test]
fn a_staged_body_written_by_a_real_broker_scans_as_source_material_with_its_own_approval() {
    let export = tempfile::tempdir().unwrap();
    let export = export.path().canonicalize().unwrap();
    let data = export.join("writer-data");
    let disk = FsVaultBroker::open(&export.join("canonical"), &data).unwrap();
    let mut request = artifact_request();
    request.provenance = provenance(4_000_000_000, &"e".repeat(64));
    let pointer = disk.stage_artifact_commit(&request).unwrap();
    let staged = format!("writer-data/artifact-candidate-{}.json", pointer.id);
    assert!(
        export.join(&staged).is_file(),
        "暂存正文就在写者数据根这一层"
    );

    let facts = read_trusted_export(&export, disk.vault_uuid()).unwrap();
    let fact = facts
        .iter()
        .find(|fact| fact.export_path.as_str() == staged)
        .expect("暂存正文有一条事实");
    assert_eq!(fact.class, TrustedExportClass::SourceMaterial);
    assert_eq!(fact.provenance.as_ref(), Some(&request.provenance));
    assert!(
        facts
            .iter()
            .filter(|fact| fact.export_path.as_str() != staged)
            .all(|fact| fact.provenance.is_none()),
        "别的写者状态文件不带批准"
    );

    let error = read_trusted_export(&export, VaultUuid::new())
        .unwrap_err()
        .to_string();
    assert!(error.contains("暂存正文没有原批准"), "{error}");
    fs::write(
        data.join(format!("artifact-candidate-{}.json", "0".repeat(64))),
        br#"{"artifacts":[]}"#,
    )
    .unwrap();
    let error = read_trusted_export(&export, disk.vault_uuid())
        .unwrap_err()
        .to_string();
    assert!(error.contains("暂存正文没有原批准"), "{error}");
}
