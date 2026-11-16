//! Broker 能力 trait，以及两种适配器共用的尝试产物 manifest 核验。

use studiolo_core::{Revision, VaultPath, VaultUuid};

use crate::{
    ArtifactCommit, ArtifactCommitReceipt, BrokerError, CommittedArtifact, Mutation,
    MutationReceipt, Snapshot, SnapshotId, SnapshotInput, VaultObject, sha256,
    types::AttemptManifest,
};

/// canonical Vault 对象的读取能力。
pub trait VaultReader: Send + Sync {
    /// 返回 Vault 持久身份。
    fn vault_uuid(&self) -> VaultUuid;

    /// 读取无链接的普通文件对象，确认读取期间对象稳定后给出其 revision。
    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError>;
}

/// 快照创建能力。
pub trait VaultSnapshotter: Send + Sync {
    /// 把精确 revision 捕获为不可变、按内容寻址的快照。
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError>;
}

/// 已捕获快照的读取能力。
pub trait SnapshotReader: Send + Sync {
    /// 加载快照，不暴露宿主路径。
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError>;
}

/// 单对象整篇变更与尝试产物发布的写能力；批量、流式、文件操作等写入口另有专门接口。
pub trait VaultWriter: Send + Sync {
    /// 执行一次幂等、受 CAS 保护的整篇内容变更。
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError>;

    /// 发布尝试产物，最后写入 `manifest.json`。
    fn commit_attempt_artifacts(
        &mut self,
        commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError>;
}

/// 组合根使用的完整 Broker 接口。
pub trait VaultBroker: VaultReader + VaultSnapshotter + SnapshotReader + VaultWriter {}

impl<T> VaultBroker for T where T: VaultReader + VaultSnapshotter + SnapshotReader + VaultWriter {}

pub(crate) fn verify_manifest(
    broker: &impl VaultReader,
    commit: &ArtifactCommit,
    manifest: &AttemptManifest,
    existing: &VaultObject,
) -> Result<(), BrokerError> {
    if manifest.format_version != 2
        || manifest.provenance.as_ref() != Some(&commit.provenance)
        || manifest.task_id != commit.task_id.to_string()
        || manifest.attempt_id != commit.attempt_id.to_string()
        || manifest.fence != commit.fence.get()
        || manifest.audit_intent != commit.audit_intent
        || manifest.artifacts.len() != commit.artifacts.len()
    {
        return Err(BrokerError::ArtifactConflict(existing.path.clone()));
    }
    for (record, candidate) in manifest.artifacts.iter().zip(&commit.artifacts) {
        let expected_path = VaultPath::parse(format!(
            "sources/attempts/{}/{}/{}",
            commit.task_id, commit.attempt_id, candidate.name
        ))
        .map_err(|_| BrokerError::ArtifactConflict(existing.path.clone()))?;
        let expected_revision =
            Revision::compute(broker.vault_uuid(), &expected_path, &candidate.content);
        if record.path != expected_path
            || record.revision != expected_revision
            || record.sha256 != sha256(&candidate.content)
        {
            return Err(BrokerError::ArtifactConflict(expected_path));
        }
        let actual = broker.read(&record.path)?;
        if actual.revision != expected_revision || sha256(&actual.content) != record.sha256 {
            return Err(BrokerError::ArtifactConflict(record.path.clone()));
        }
    }
    if broker.read(&existing.path)?.revision != existing.revision {
        return Err(BrokerError::UnstableRead(existing.path.clone()));
    }
    Ok(())
}

pub(crate) fn verify_artifact_records(
    broker: &impl VaultReader,
    artifacts: &[CommittedArtifact],
) -> Result<(), BrokerError> {
    for artifact in artifacts {
        let actual = broker.read(&artifact.path)?;
        if actual.revision != artifact.revision || sha256(&actual.content) != artifact.sha256 {
            return Err(BrokerError::ArtifactConflict(artifact.path.clone()));
        }
    }
    Ok(())
}

/// 采样系统 UTC。新的暂存和正文写入用它判断期限，不接受调用方自报的时钟。
pub(crate) fn system_now_ms() -> Result<i64, BrokerError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| BrokerError::InvalidArtifact("系统时钟无效".into()))?;
    i64::try_from(now.as_millis()).map_err(|_| BrokerError::InvalidArtifact("系统时钟无效".into()))
}

pub(crate) fn reject_expired(commit: &ArtifactCommit) -> Result<(), BrokerError> {
    if commit.provenance.is_live_at(system_now_ms()?) {
        Ok(())
    } else {
        Err(BrokerError::InvalidArtifact(
            "产物留存期限已过，拒绝新的暂存与正文写入".into(),
        ))
    }
}

/// 已有清单的重放判定。旧版本或缺少来源时失败关闭，不把缺字段升成完整范围。
pub(crate) fn accept_existing_manifest(
    manifest: &AttemptManifest,
    commit: &ArtifactCommit,
    request_fingerprint: &str,
    manifest_path: &VaultPath,
) -> Result<(), BrokerError> {
    if manifest.format_version == 1 || manifest.provenance.is_none() {
        return Err(BrokerError::InvalidArtifact(
            "旧清单缺少来源元数据，不能升为完整范围".into(),
        ));
    }
    if manifest.format_version != 2 {
        return Err(BrokerError::ArtifactConflict(manifest_path.clone()));
    }
    if manifest.provenance.as_ref() != Some(&commit.provenance)
        || manifest.idempotency_key != commit.idempotency_key
        || manifest.request_fingerprint != request_fingerprint
    {
        return Err(BrokerError::IdempotencyConflict);
    }
    Ok(())
}

pub(crate) fn fresh_manifest(
    commit: &ArtifactCommit,
    request_fingerprint: String,
    artifacts: Vec<CommittedArtifact>,
) -> Result<AttemptManifest, BrokerError> {
    // 这里不再查期限：调用方在写第一篇正文之前已经判定过，正文写完之后再拒绝清单只会留下孤儿正文。
    Ok(AttemptManifest {
        format_version: 2,
        task_id: commit.task_id.to_string(),
        attempt_id: commit.attempt_id.to_string(),
        fence: commit.fence.get(),
        audit_intent: commit.audit_intent.clone(),
        idempotency_key: commit.idempotency_key.clone(),
        request_fingerprint,
        artifacts,
        provenance: Some(commit.provenance.clone()),
    })
}
