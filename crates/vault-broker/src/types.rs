//! 跨接口共用的值类型、Broker 私有持久记录的格式，以及快照标识与摘要的派生。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use studiolo_core::{AttemptId, Fence, Revision, TaskId, VaultPath, VaultUuid};

use crate::BrokerError;

/// 单次变更的比较并交换（CAS）条件。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precondition {
    /// 路径必须不存在。
    MustNotExist,
    /// 当前内容必须是此 revision。
    Match(Revision),
}

/// 一次普通的 canonical Vault 变更。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mutation {
    /// 目标逻辑路径。
    pub path: VaultPath,
    /// 完整的替换内容。
    pub content: Vec<u8>,
    /// 要求的先前状态。
    pub precondition: Precondition,
    /// 调用方提供的稳定重放键。
    pub idempotency_key: String,
}

impl Mutation {
    /// 构造仅创建（create-only）的变更。
    #[must_use]
    pub fn create(path: VaultPath, content: Vec<u8>, idempotency_key: impl Into<String>) -> Self {
        Self {
            path,
            content,
            precondition: Precondition::MustNotExist,
            idempotency_key: idempotency_key.into(),
        }
    }

    /// 构造比较并交换（CAS）的替换变更。
    #[must_use]
    pub fn replace(
        path: VaultPath,
        content: Vec<u8>,
        expected: Revision,
        idempotency_key: impl Into<String>,
    ) -> Self {
        Self {
            path,
            content,
            precondition: Precondition::Match(expected),
            idempotency_key: idempotency_key.into(),
        }
    }
}

/// 稳定读取返回的 canonical 对象。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultObject {
    /// 逻辑路径。
    pub path: VaultPath,
    /// 完整内容。
    pub content: Vec<u8>,
    /// 绑定 Vault 身份、路径与内容的 revision。
    pub revision: Revision,
}

/// 一次变更的持久结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationReceipt {
    /// 被修改的逻辑路径。
    pub path: VaultPath,
    /// 写入后的 revision。
    pub revision: Revision,
    /// 原对象存在时，为已存入 L0 历史的原 revision。
    pub previous_revision: Option<Revision>,
}

/// 请求纳入快照的路径与精确 revision。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotInput {
    /// 源逻辑路径。
    pub path: VaultPath,
    /// 必须仍是当前版本的 revision。
    pub expected_revision: Revision,
}

/// 按内容寻址的快照标识。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotId(#[serde(deserialize_with = "deserialize_snapshot_id")] String);

fn deserialize_snapshot_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "snapshot id must be 64 lowercase hexadecimal characters",
        ))
    }
}

impl SnapshotId {
    /// 返回十六进制标识。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// 面向 runner 的快照中的一个不可变文件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFile {
    /// Vault 逻辑路径；不暴露宿主路径。
    pub path: VaultPath,
    /// 捕获时的 revision。
    pub revision: Revision,
    /// 捕获的字节。
    pub content: Vec<u8>,
}

/// 不可变、持久化的输入快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// 由 Vault 身份与文件清单（路径、revision）确定性派生的标识。
    pub id: SnapshotId,
    /// 捕获时的 Vault 身份。
    pub vault_uuid: VaultUuid,
    /// 按逻辑路径排序的文件。
    pub files: Vec<SnapshotFile>,
}

/// 一个由 runner 产出的产物候选。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCandidate {
    /// 位于尝试（attempt）目录下的相对名称。
    pub name: VaultPath,
    /// 从 runner 自有临时存储复制来的候选字节。
    pub content: Vec<u8>,
}

/// 发布一次尝试（attempt）全部产物的请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCommit {
    /// 所属任务。
    pub task_id: TaskId,
    /// 所属尝试。
    pub attempt_id: AttemptId,
    /// 该尝试当前的 fence。
    pub fence: Fence,
    /// 已追加的审计意图（audit intent）标识。
    pub audit_intent: String,
    /// 稳定重放键。
    pub idempotency_key: String,
    /// 候选产物；所有 canonical 目标路径都由 Broker 派生。
    pub artifacts: Vec<ArtifactCandidate>,
}

/// 尝试 manifest 中记录的一个产物。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedArtifact {
    /// canonical Vault 路径。
    pub path: VaultPath,
    /// 绑定的 Vault revision。
    pub revision: Revision,
    /// 内容的普通 SHA-256 摘要（小写十六进制，不绑定 Vault 与路径）。
    pub sha256: String,
}

/// 产物提交（manifest 最后写入）的持久结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCommitReceipt {
    /// manifest 路径，即提交点。
    pub manifest_path: VaultPath,
    /// manifest 自身的 revision。
    pub manifest_revision: Revision,
    /// 已发布的产物。
    pub artifacts: Vec<CommittedArtifact>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct AttemptManifest {
    pub(crate) format_version: u32,
    pub(crate) task_id: String,
    pub(crate) attempt_id: String,
    pub(crate) fence: u64,
    pub(crate) audit_intent: String,
    pub(crate) idempotency_key: String,
    pub(crate) request_fingerprint: String,
    pub(crate) artifacts: Vec<CommittedArtifact>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct VaultIdentity {
    pub(crate) format_version: u32,
    pub(crate) vault_uuid: VaultUuid,
}

impl VaultIdentity {
    pub(crate) fn decode(bytes: &[u8]) -> Result<VaultUuid, BrokerError> {
        let identity: Self = serde_json::from_slice(bytes)
            .map_err(|error| BrokerError::json("parse Vault identity", error))?;
        if identity.format_version != 1 {
            return Err(BrokerError::CorruptIdentity);
        }
        Ok(identity.vault_uuid)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MutationReplay {
    pub(crate) fingerprint: String,
    pub(crate) expected_revision: Revision,
    pub(crate) receipt: Option<MutationReceipt>,
}

pub(crate) fn snapshot_identity(
    vault_uuid: VaultUuid,
    files: &[SnapshotFile],
) -> Result<SnapshotId, BrokerError> {
    let manifest: Vec<(&VaultPath, Revision)> = files
        .iter()
        .map(|file| (&file.path, file.revision))
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(b"studiolo-snapshot-v1\0");
    hasher.update(vault_uuid.to_string().as_bytes());
    hasher.update(
        serde_json::to_vec(&manifest)
            .map_err(|error| BrokerError::json("serialize snapshot manifest", error))?,
    );
    Ok(SnapshotId(hex(&hasher.finalize())))
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
