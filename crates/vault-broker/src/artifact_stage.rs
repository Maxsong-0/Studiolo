//! 候选正文仅存 Broker 私有数据根；控制面只持内容寻址指针。
use crate::writer::disk;
use crate::{ArtifactCommit, BrokerError, FsVaultBroker, MemoryVaultBroker, VaultUuid, sha256};
use serde::{Deserialize, Serialize};
use std::path::Path;

const LIMIT: usize = 8 * 1024 * 1024;

/// 可持久化的候选引用；既不包含正文，也不暴露环境路径。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagedArtifactCommit {
    /// 绑定 Vault 身份和原始候选字节的逻辑标识。
    pub id: String,
    /// 私有候选记录的 SHA-256。
    pub sha256: String,
}

/// 与 canonical 写权限分开的私有候选保存能力。
pub trait ArtifactStager: Send + Sync {
    /// 固定完整候选，返回正文以外的控制面指针。
    fn stage_artifact_commit(
        &self,
        plan: &ArtifactCommit,
    ) -> Result<StagedArtifactCommit, BrokerError>;
    /// 核验 Vault 绑定与完整摘要后读取候选。
    fn load_staged_artifact_commit(
        &self,
        pointer: &StagedArtifactCommit,
    ) -> Result<ArtifactCommit, BrokerError>;
}

fn encode(plan: &ArtifactCommit) -> Result<Vec<u8>, BrokerError> {
    if plan.artifacts.len() > 64 || plan.audit_intent.is_empty() || plan.idempotency_key.is_empty()
    {
        return Err(BrokerError::InvalidArtifact("候选数量或身份无效".into()));
    }
    let size = plan
        .artifacts
        .iter()
        .try_fold(0usize, |sum, item| sum.checked_add(item.content.len()))
        .ok_or(BrokerError::WriterLimitExceeded)?;
    if size > 4 * 1024 * 1024 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let bytes = serde_json::to_vec(plan).map_err(|_| BrokerError::CorruptWriterState)?;
    if bytes.len() > LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(bytes)
}
fn pointer(uuid: VaultUuid, bytes: &[u8]) -> StagedArtifactCommit {
    let digest = sha256(bytes);
    StagedArtifactCommit {
        id: sha256(format!("studiolo-artifact-stage-v1:{uuid}:{digest}").as_bytes()),
        sha256: digest,
    }
}
fn decode(
    uuid: VaultUuid,
    reference: &StagedArtifactCommit,
    bytes: &[u8],
) -> Result<ArtifactCommit, BrokerError> {
    if !disk::hex(&reference.id)
        || !disk::hex(&reference.sha256)
        || &pointer(uuid, bytes) != reference
    {
        return Err(BrokerError::CorruptWriterState);
    }
    let plan: ArtifactCommit = disk::decode(bytes)?;
    if encode(&plan)? != bytes {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(plan)
}
fn name(reference: &StagedArtifactCommit) -> Result<String, BrokerError> {
    if !disk::hex(&reference.id) || !disk::hex(&reference.sha256) {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(format!("artifact-candidate-{}.json", reference.id))
}
impl ArtifactStager for FsVaultBroker {
    fn stage_artifact_commit(
        &self,
        plan: &ArtifactCommit,
    ) -> Result<StagedArtifactCommit, BrokerError> {
        self.writer.check(&self.root, &self.data)?;
        let bytes = encode(plan)?;
        let reference = pointer(self.vault_uuid, &bytes);
        let path = name(&reference)?;
        if let Some(existing) = disk::read(&self.data, &path, LIMIT)? {
            decode(self.vault_uuid, &reference, &existing)?;
        } else {
            let mut count = 0usize;
            let mut total = bytes.len() as u64;
            for entry in self
                .data
                .entries()
                .map_err(|e| BrokerError::io("enumerate artifact candidates", e))?
            {
                let entry = entry.map_err(|e| BrokerError::io("inspect artifact candidate", e))?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("artifact-candidate-")
                {
                    count += 1;
                    let metadata = entry
                        .metadata()
                        .map_err(|e| BrokerError::io("stat artifact candidate", e))?;
                    total = total
                        .checked_add(metadata.len())
                        .ok_or(BrokerError::WriterLimitExceeded)?;
                }
            }
            if count >= 4096 || total > 64 * 1024 * 1024 {
                return Err(BrokerError::WriterLimitExceeded);
            }
            Self::atomic_write_dir_checked(&self.data, Path::new(&path), &bytes, true, || {
                self.writer.check(&self.root, &self.data)
            })?;
        }
        self.load_staged_artifact_commit(&reference)?;
        Ok(reference)
    }
    fn load_staged_artifact_commit(
        &self,
        reference: &StagedArtifactCommit,
    ) -> Result<ArtifactCommit, BrokerError> {
        self.writer.check(&self.root, &self.data)?;
        let bytes = disk::read(&self.data, &name(reference)?, LIMIT)?
            .ok_or(BrokerError::CorruptWriterState)?;
        let result = decode(self.vault_uuid, reference, &bytes)?;
        self.writer.check(&self.root, &self.data)?;
        Ok(result)
    }
}
impl ArtifactStager for MemoryVaultBroker {
    fn stage_artifact_commit(
        &self,
        plan: &ArtifactCommit,
    ) -> Result<StagedArtifactCommit, BrokerError> {
        let bytes = encode(plan)?;
        let reference = pointer(self.vault_uuid, &bytes);
        let mut stages = self
            .artifact_stages
            .lock()
            .map_err(|_| crate::memory_state_error())?;
        if !stages.contains_key(&reference.id)
            && (stages.len() >= 4096
                || stages.values().map(Vec::len).sum::<usize>() + bytes.len() > 64 * 1024 * 1024)
        {
            return Err(BrokerError::WriterLimitExceeded);
        }
        stages.insert(reference.id.clone(), bytes);
        Ok(reference)
    }
    fn load_staged_artifact_commit(
        &self,
        reference: &StagedArtifactCommit,
    ) -> Result<ArtifactCommit, BrokerError> {
        let stages = self
            .artifact_stages
            .lock()
            .map_err(|_| crate::memory_state_error())?;
        let bytes = stages
            .get(&reference.id)
            .ok_or(BrokerError::CorruptWriterState)?;
        decode(self.vault_uuid, reference, bytes)
    }
}
