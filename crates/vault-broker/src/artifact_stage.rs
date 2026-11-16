//! 候选正文仅存 Broker 私有数据根；控制面只持内容寻址指针。
use crate::broker::{expired_before_start, reject_expired};
use crate::writer::disk;
use crate::{ArtifactCommit, BrokerError, FsVaultBroker, MemoryVaultBroker, VaultUuid, sha256};
use serde::{Deserialize, Serialize};
use std::path::Path;
use studiolo_core::ArtifactProvenance;

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
    /// 只计算指针，不写新的暂存。精确重放用它核对，避免过期后重写正文。
    fn staged_reference(&self, plan: &ArtifactCommit) -> Result<StagedArtifactCommit, BrokerError>;
    /// 删除已经核对过的原暂存。文件不存在视为已清理；身份不符则拒绝。
    fn discard_staged_artifact_commit(
        &self,
        pointer: &StagedArtifactCommit,
    ) -> Result<(), BrokerError>;
    /// 这份提交是不是「原批准已经过期，而且一个字节都还没写出去」（没有清单，也没有任何一篇正文）。
    ///
    /// 按 Broker 自己的时钟判断，只读。回答「是」时调用方才可以把这次提交落成失败并丢掉暂存。
    /// 默认回答「不是」：证明不了就保持待恢复，不走失败出口。
    fn expired_before_start(&self, plan: &ArtifactCommit) -> Result<bool, BrokerError> {
        let _ = plan;
        Ok(false)
    }
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
const NAME_PREFIX: &str = "artifact-candidate-";
const NAME_SUFFIX: &str = ".json";

/// 导出扫描用：文件名是不是以暂存正文的前缀开头。只看前缀：后缀不对的也要交给
/// `staged_provenance` 去拒绝，不能让它漏到别的归类里。
pub(crate) fn is_staged_name(file_name: &str) -> bool {
    file_name.starts_with(NAME_PREFIX)
}

/// 导出扫描用：按文件名和内容核对一份暂存正文（和 `load_staged_artifact_commit` 同一套核对），
/// 取出它自带的原批准。升级前留下的暂存没有这一项，这里读不出来，调用方按拒绝处理。
pub(crate) fn staged_provenance(
    uuid: VaultUuid,
    file_name: &str,
    bytes: &[u8],
) -> Result<ArtifactProvenance, BrokerError> {
    let id = file_name
        .strip_prefix(NAME_PREFIX)
        .and_then(|rest| rest.strip_suffix(NAME_SUFFIX))
        .ok_or(BrokerError::CorruptWriterState)?;
    let reference = StagedArtifactCommit {
        id: id.to_owned(),
        sha256: sha256(bytes),
    };
    Ok(decode(uuid, &reference, bytes)?.provenance)
}

fn name(reference: &StagedArtifactCommit) -> Result<String, BrokerError> {
    if !disk::hex(&reference.id) || !disk::hex(&reference.sha256) {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(format!("{NAME_PREFIX}{}{NAME_SUFFIX}", reference.id))
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
            reject_expired(plan)?;
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
    fn staged_reference(&self, plan: &ArtifactCommit) -> Result<StagedArtifactCommit, BrokerError> {
        Ok(pointer(self.vault_uuid, &encode(plan)?))
    }
    fn discard_staged_artifact_commit(
        &self,
        reference: &StagedArtifactCommit,
    ) -> Result<(), BrokerError> {
        self.writer.check(&self.root, &self.data)?;
        let path = name(reference)?;
        if let Some(bytes) = disk::read(&self.data, &path, LIMIT)? {
            decode(self.vault_uuid, reference, &bytes)?;
            self.data
                .remove_file(Path::new(&path))
                .map_err(|error| BrokerError::io("remove staged artifact", error))?;
        }
        self.writer.check(&self.root, &self.data)?;
        Ok(())
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
    fn expired_before_start(&self, plan: &ArtifactCommit) -> Result<bool, BrokerError> {
        self.writer.check(&self.root, &self.data)?;
        // 有没收口的写入时说不清「写没写」：不回答「是」。
        if self.write_status()?.unresolved {
            return Ok(false);
        }
        let manifest = Self::manifest_path(plan.task_id, plan.attempt_id)?;
        expired_before_start(plan, &manifest, |path| {
            Ok(self.inspect_existing(path)?.is_some())
        })
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
        if !stages.contains_key(&reference.id) {
            reject_expired(plan)?;
            if stages.len() >= 4096
                || stages.values().map(Vec::len).sum::<usize>() + bytes.len() > 64 * 1024 * 1024
            {
                return Err(BrokerError::WriterLimitExceeded);
            }
        }
        stages.insert(reference.id.clone(), bytes);
        Ok(reference)
    }
    fn staged_reference(&self, plan: &ArtifactCommit) -> Result<StagedArtifactCommit, BrokerError> {
        Ok(pointer(self.vault_uuid, &encode(plan)?))
    }
    fn discard_staged_artifact_commit(
        &self,
        reference: &StagedArtifactCommit,
    ) -> Result<(), BrokerError> {
        let mut stages = self
            .artifact_stages
            .lock()
            .map_err(|_| crate::memory_state_error())?;
        if let Some(bytes) = stages.get(&reference.id) {
            decode(self.vault_uuid, reference, bytes)?;
            stages.remove(&reference.id);
        }
        Ok(())
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
    fn expired_before_start(&self, plan: &ArtifactCommit) -> Result<bool, BrokerError> {
        let manifest = FsVaultBroker::manifest_path(plan.task_id, plan.attempt_id)?;
        expired_before_start(plan, &manifest, |path| {
            Ok(self.inspect_existing(path)?.is_some())
        })
    }
}
