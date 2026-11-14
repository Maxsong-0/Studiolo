//! 最终记录绑定原作业、封存摘要和数据快照；本类型自身不宣称远端已提交。
mod codec;
mod receipt;
use super::{
    restic::SnapshotRef,
    state::{JobKey, RemoteVerification},
};
use crate::checkpoint::{
    manifest::CheckpointManifest,
    staging::{SealedCheckpoint, SealedReference},
};
pub use codec::FinalManifestError;
pub use receipt::FinalManifestReceipt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use studiolo_core::checkpoint::InstantMillis;

/// 非正文最终记录的编码上限；可持久保存到 worker 状态，不能无限扩大。
pub const FINAL_MANIFEST_MAX_BYTES: usize = 64 * 1024;
/// 待远端提交的确定字节内容。字段不可修改，反序列化同样执行全部绑定检查。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FinalManifestDraft(Fields);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    schema: u32,
    key: JobKey,
    source: SealedReference,
    source_root: PathBuf,
    source_expires_at: InstantMillis,
    data_snapshot: SnapshotRef,
    manifest: CheckpointManifest,
    manifest_sha256: String,
    data_verified_at: InstantMillis,
}
impl FinalManifestDraft {
    /// 由真实封存与该数据快照的校验结果生成；提交适配器仍须核验自己持有的远端证据。
    pub fn new(
        key: JobKey,
        source: &SealedCheckpoint,
        data_snapshot: SnapshotRef,
        verification: RemoteVerification,
    ) -> Result<Self, FinalManifestError> {
        if !verification.is_recoverable() {
            return Err(FinalManifestError::Invalid);
        }
        let manifest_sha256 = source
            .inventory()
            .files()
            .find(|file| file.path().as_str() == "manifest.json")
            .ok_or(FinalManifestError::Invalid)?
            .sha256()
            .to_owned();
        let draft = Self(Fields {
            schema: 1,
            key,
            source: source.reference().clone(),
            source_root: source.ticket().transfer_source.root.clone(),
            source_expires_at: source.ticket().transfer_source.expires_at,
            data_snapshot,
            manifest: source.manifest().clone(),
            manifest_sha256,
            data_verified_at: verification.verified_at,
        });
        draft.validate()?;
        Ok(draft)
    }
    fn validate(&self) -> Result<(), FinalManifestError> {
        let f = &self.0;
        if f.schema != 1
            || f.key.checkpoint_id != f.source.checkpoint_id
            || f.manifest.checkpoint_id != f.key.checkpoint_id
            || f.data_snapshot.as_str().len() != 64
            || !codec::digest_shaped(&f.source.seal_sha256)
            || !codec::digest_shaped(&f.manifest_sha256)
            || f.manifest.content_cutoff.millis() < 0
            || f.data_verified_at < f.manifest.content_cutoff
            || f.data_verified_at >= f.source_expires_at
        {
            return Err(FinalManifestError::Invalid);
        }
        f.manifest
            .validate()
            .map_err(|_| FinalManifestError::Invalid)?;
        let path = f.source_root.to_str().ok_or(FinalManifestError::Invalid)?;
        if !f.source_root.is_absolute()
            || path.len() > 4096
            || path == "/"
            || path.ends_with('/')
            || path.contains("//")
            || path.split('/').any(|part| matches!(part, "." | ".."))
            || path.chars().any(char::is_control)
        {
            return Err(FinalManifestError::Invalid);
        }
        if codec::digest(&codec::encode(&f.manifest)?) != f.manifest_sha256 {
            return Err(FinalManifestError::Invalid);
        }
        self.encode()?;
        Ok(())
    }
    /// 严格读取有界原字节；拒绝非规范编码，确保待提交摘要不会在重试时变化。
    pub fn decode(bytes: &[u8]) -> Result<Self, FinalManifestError> {
        if bytes.len() > FINAL_MANIFEST_MAX_BYTES {
            return Err(FinalManifestError::TooLarge);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| FinalManifestError::Invalid)?;
        if value.encode()? != bytes {
            return Err(FinalManifestError::Invalid);
        }
        Ok(value)
    }
    /// 远端上传和本地待提交状态使用完全相同的规范 JSON 字节。
    pub fn encode(&self) -> Result<Vec<u8>, FinalManifestError> {
        codec::encode(&self.0)
    }
    /// 上述确定字节的完整摘要。
    pub fn sha256(&self) -> Result<String, FinalManifestError> {
        Ok(codec::digest(&self.encode()?))
    }
    /// 原作业键，不由内容摘要代替。
    #[must_use]
    pub fn key(&self) -> &JobKey {
        &self.0.key
    }
    /// 原外部封存引用。
    #[must_use]
    pub fn source(&self) -> &SealedReference {
        &self.0.source
    }
    /// 数据快照内部的原封存路径，不能作为恢复目标路径。
    #[must_use]
    pub fn source_root(&self) -> &Path {
        &self.0.source_root
    }
    /// 原传输副本的期限，不因最终记录重试而延期。
    #[must_use]
    pub fn source_expires_at(&self) -> InstantMillis {
        self.0.source_expires_at
    }
    /// 已通过完整核验的数据快照；与最终记录自身快照分开。
    #[must_use]
    pub fn data_snapshot(&self) -> &SnapshotRef {
        &self.0.data_snapshot
    }
    /// 原封存中已经配对的 checkpoint 清单。
    #[must_use]
    pub fn manifest(&self) -> &CheckpointManifest {
        &self.0.manifest
    }
    /// 完整数据校验发生的实际时刻。
    #[must_use]
    pub fn data_verified_at(&self) -> InstantMillis {
        self.0.data_verified_at
    }
}
impl<'de> Deserialize<'de> for FinalManifestDraft {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Self(Fields::deserialize(deserializer)?);
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}
