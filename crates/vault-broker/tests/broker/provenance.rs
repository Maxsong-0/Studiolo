//! 清单第 2 版保存显式来源；旧清单缺字段时不能升成完整范围，过期正文不再写入。
use super::{artifact_request, broker};
use std::fs;
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    SourcePolicyRef,
};
use studiolo_core::{TrustedExportClass, VaultPath, VaultUuid};
use studiolo_vault_broker::{
    ArtifactCandidate, ArtifactCommit, ArtifactStager, BrokerError, FsVaultBroker,
    MemoryVaultBroker, Mutation, VaultReader, VaultWriter, read_trusted_export,
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

/// 过期提交的两种结局，磁盘和内存两个实现走同一份断言。
fn started_commit_finishes_and_untouched_one_stays_rejected<B>(
    broker: &mut B,
    request: &ArtifactCommit,
) where
    B: VaultReader + VaultWriter + ArtifactStager,
{
    // 一个字节都没写：能证实「过期且没开始」，提交照旧被拒，被拒之后仍然什么都没留下。
    assert!(broker.expired_before_start(request).unwrap());
    assert!(matches!(
        broker.commit_attempt_artifacts(request.clone()),
        Err(BrokerError::InvalidArtifact(ref message)) if message.contains("留存期限已过")
    ));
    assert!(broker.expired_before_start(request).unwrap());

    // 模拟上次进程在过期之前已经写出第一篇正文、然后没了。
    let first = &request.artifacts[0];
    let target = VaultPath::parse(format!(
        "sources/attempts/{}/{}/{}",
        request.task_id,
        request.attempt_id,
        first.name.as_str()
    ))
    .unwrap();
    broker
        .mutate(Mutation::create(
            target,
            first.content.clone(),
            "earlier-run",
        ))
        .unwrap();
    assert!(!broker.expired_before_start(request).unwrap());

    // 已经开始的提交写完为止：剩下那篇和清单都写出来，不留没有清单的半截目录。
    let receipt = broker.commit_attempt_artifacts(request.clone()).unwrap();
    assert_eq!(receipt.artifacts.len(), 2);
    for artifact in &receipt.artifacts {
        broker.read(&artifact.path).unwrap();
    }
    broker.read(&receipt.manifest_path).unwrap();
    // 重放得到同一份回执；有了清单更谈不上「没开始」。
    assert_eq!(
        broker.commit_attempt_artifacts(request.clone()).unwrap(),
        receipt
    );
    assert!(!broker.expired_before_start(request).unwrap());
}

#[test]
fn expiry_only_stops_commits_that_have_not_written_anything_yet() {
    let mut request = artifact_request();
    request.provenance = provenance(1, &"e".repeat(64));
    request.artifacts.push(ArtifactCandidate {
        name: VaultPath::parse("second.md").unwrap(),
        content: b"second body".to_vec(),
    });
    let (_vault, _data, mut disk) = broker();
    started_commit_finishes_and_untouched_one_stays_rejected(&mut disk, &request);
    let mut memory = MemoryVaultBroker::new();
    started_commit_finishes_and_untouched_one_stays_rejected(&mut memory, &request);

    // 批准还有效的提交谈不上「过期」，哪怕一个字节都没写。
    let live = artifact_request();
    assert!(
        !MemoryVaultBroker::new()
            .expired_before_start(&live)
            .unwrap()
    );
    let (_vault, _data, disk) = broker();
    assert!(!disk.expired_before_start(&live).unwrap());
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
