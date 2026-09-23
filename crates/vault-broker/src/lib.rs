//! 以目录能力句柄为根访问 canonical Studiolo Vault。
//!
//! 本 crate 是唯一获准修改 canonical Vault 文件的生产 crate。
//! 调用方只传入逻辑 [`VaultPath`] 值，绝不传入宿主（ambient）路径。

mod numbering;
pub use numbering::NumberingScanner;
mod artifact_stage;
pub use artifact_stage::{ArtifactStager, StagedArtifactCommit};
mod batch;
pub use batch::{
    BatchChange, BatchDiff, BatchEdit, BatchPreview, BatchReceipt, BatchRequest, BatchWriter,
    TextPatch,
};
mod blob;
mod stream;
mod sync_observation;
pub use stream::{PreparedStream, StreamMutation};
pub use sync_observation::{
    CanonicalSyncFile, CanonicalSyncObservation, CanonicalSyncPresence, CanonicalSyncSurvey,
    SyncReadRule, SyncReadSelection,
};
mod canonical;
mod capture;
mod check_precondition;
mod content_audit;
pub(crate) mod file_operation;
mod gateway_seal;
pub use gateway_seal::{GatewayCallReceipt, GatewayCallSeal, GatewayPublisher};
mod history;
#[cfg(test)]
mod l0_tests;
mod maintenance;
pub use file_operation::{
    FileEffect, FileOperationAuditCandidate, FileOperationAuditEvent, FileOperationKind,
    FileOperationLimits, FileOperationMetadata, FileOperationOutcome, FileOperationPhase,
    FileOperationPreview, FileOperationReceipt, FileOperationRequest, FileOperationScopeHash,
    FileOperationStatus, FileOperationTicket, FilePresence, TombstoneEntry, TombstonePage,
};
mod memory_write;
pub use memory_write::{MemoryEntryRequest, MemoryPreview, MemoryWriter};
mod memory_mutation;
mod memory_namespace;
mod mutation;
mod readonly;
mod recorded;
mod recovery;
mod replay_read;
mod restore;
mod writer;

pub use blob::{BlobIdentity, BlobPruneReport, BlobSource, StreamLimits};
pub use canonical::{CanonicalRead, CanonicalReadVault};
pub use capture::{CanonicalCapture, CaptureStamp};
pub use content_audit::{
    ContentAuditCandidate, ContentAuditEvent, ContentAuditOutcome, ContentAuditPhase,
    ContentAuditStatus,
};
pub use history::{
    HistoryContent, HistoryEntry, HistoryPage, HistoryPruneReport, HistoryRetention,
};
pub use maintenance::{
    MaintenanceActor, MaintenanceAuditCandidate, MaintenanceAuditEvent, MaintenanceCurrent,
    MaintenanceDecision, MaintenanceFileOperation, MaintenanceOutcome, MaintenancePending,
    MaintenanceReceipt, MaintenanceSecondaryTarget, MaintenanceStatus, MaintenanceTarget,
    MaintenanceTargetKind, MaintenanceTicket, MaintenanceUnavailableReason,
};
pub use readonly::{
    LocalReadFile, LocalReadObservation, LocalReadPresence, LocalReadSurvey, ReadBatch, ReadEntry,
    ReadEntryKind, ReadNote, ReadOnlyVault, ScanLimits,
};
pub use recorded::{
    MutationActor, MutationKind, MutationMetadata, PendingRecordedRestore, RecordedRestore,
};
pub use recovery::{
    PendingDecision, PendingMutation, ReconciliationOutcome, ReconciliationReceipt,
    StagePruneReport, WriterPhase,
};
pub use restore::{
    LineageState, MaintenanceCoverage, NewRootRebind, PhysicalId, RebindCoverage, RestoreError,
    RestoreLineage, RestoreReceipt, read_restore_lineage, rebind_new_roots,
};
pub use writer::WriterStatus;

use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use cap_std::fs::{Dir, MetadataExt as _, OpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use studiolo_core::{AttemptId, Fence, Revision, TaskId, VaultPath, VaultUuid};

#[cfg(target_os = "linux")]
use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, openat2, renameat_with};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Vault 边界对外暴露的失败类型。
#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    /// 同一 canonical 根已有存活写者。
    #[error("canonical writer is busy")]
    WriterBusy,
    /// canonical、data 或不可变控制对象与已登记配对不符。
    #[error("writer storage binding mismatch")]
    WriterBindingMismatch,
    /// 写者持久记录缺失、格式不合法或不满足协议不变量。
    #[error("corrupt writer state")]
    CorruptWriterState,
    /// 未决安装或旧未知操作阻止新的 canonical 安装。
    #[error("canonical mutation is unresolved")]
    MutationUnresolved,
    /// 原请求已明确取消，重放不会再次尝试安装。
    #[error("canonical mutation was cancelled")]
    MutationCancelled,
    /// 未知安装已封存证据并终止重放，不代表原请求曾成功或失败。
    #[error("canonical mutation remains indeterminate: {evidence_id}")]
    MutationIndeterminate {
        /// 私有协调证据 ID，不包含正文或主机路径。
        evidence_id: String,
    },
    /// 写者协议触及的输入或持久记录超过固定预算。
    #[error("writer storage or input limit exceeded")]
    WriterLimitExceeded,
    /// 捕获所属实例或已确认提交水位变化，候选视图不能发布。
    #[error("canonical capture changed")]
    CaptureChanged,
    /// 扫描出现安全诊断，不能把部分结果当作完整捕获。
    #[error("canonical capture is incomplete")]
    IncompleteCapture,
    /// 受保护的控制路径（或隐藏分量）被送入普通变更、历史、编号等业务接口。
    #[error("protected Vault path: {0}")]
    ProtectedPath(VaultPath),
    /// 历史游标语法错误或不属于本路径。
    #[error("invalid history cursor")]
    InvalidHistoryCursor,
    /// 历史目录已变化，调用方须重新从第一页开始。
    #[error("history changed")]
    HistoryChanged,
    /// 请求的对象不存在。
    #[error("Vault object not found: {0}")]
    NotFound(VaultPath),
    /// 比较并交换（CAS）前置条件不匹配。
    #[error("revision conflict at {path}: expected {expected}, found {actual:?}")]
    RevisionConflict {
        /// 发生冲突的逻辑路径。
        path: VaultPath,
        /// 要求的 revision；要求目标不存在（创建）时为 `absence`。
        expected: String,
        /// Broker 实际观察到的 revision。
        actual: Option<Revision>,
    },
    /// 路径分量不是安全节点：符号链接、硬链接、特殊文件、跨设备、隐藏或非 UTF-8 名称等。
    #[error("unsafe filesystem node at {path}: {reason}")]
    UnsafeNode {
        /// 被拒绝的逻辑路径。
        path: VaultPath,
        /// 不含敏感信息的固定原因。
        reason: &'static str,
    },
    /// 该拼写在常见文件系统上会与已有名称互为别名（如仅大小写或 Unicode 规范化形式不同）。
    #[error("path collision between {requested} and existing {existing}")]
    PathCollision {
        /// 请求的逻辑路径。
        requested: VaultPath,
        /// 已存在的逻辑拼写。
        existing: String,
    },
    /// 幂等键（或同类重放绑定）被用于不同的输入。
    #[error("idempotency key was reused with different input")]
    IdempotencyConflict,
    /// 请求不合法：名称沿用自产物提交，现也用于快照、维护、文件操作、编号等输入校验。
    #[error("invalid artifact commit: {0}")]
    InvalidArtifact(String),
    /// 已有产物或 manifest 与本次提交不一致（如同一提交重试时遇到内容不同的孤儿产物）。
    #[error("artifact already exists with different content: {0}")]
    ArtifactConflict(VaultPath),
    /// 持久化的 Vault 身份记录格式不合法。
    #[error("corrupt Vault identity")]
    CorruptIdentity,
    /// 已批准的 canonical 根不再解析到当初固定的目录。
    #[error("canonical Vault root changed or is unavailable")]
    RootChanged,
    /// 有界读取超出调用方给定的字节预算。
    #[error("read limit of {max_bytes} bytes exceeded at {path}")]
    ReadLimitExceeded {
        /// 字节被拒收的逻辑路径。
        path: VaultPath,
        /// 允许的最大内容字节数，读取期间的增长也计入。
        max_bytes: u64,
    },
    /// 已存快照的元数据或内容未通过完整性校验。
    #[error("corrupt snapshot record")]
    CorruptSnapshot,
    /// 读取、扫描或快照捕获期间源对象发生了变化（前后复核不一致）。
    #[error("source changed while snapshotting: {0}")]
    UnstableRead(VaultPath),
    /// 操作系统调用失败；错误中不暴露宿主路径。
    #[error("Vault I/O failed during {operation}: {source}")]
    Io {
        /// 稳定的操作标签。
        operation: &'static str,
        /// 底层操作系统错误。
        #[source]
        source: std::io::Error,
    },
    /// 持久记录的 JSON 解析失败（序列化失败也归入此类）。
    #[error("invalid durable record during {operation}: {source}")]
    Json {
        /// 稳定的操作标签。
        operation: &'static str,
        /// JSON 错误。
        #[source]
        source: serde_json::Error,
    },
}

impl BrokerError {
    fn io(operation: &'static str, source: std::io::Error) -> Self {
        Self::Io { operation, source }
    }

    fn json(operation: &'static str, source: serde_json::Error) -> Self {
        Self::Json { operation, source }
    }
}

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
struct AttemptManifest {
    format_version: u32,
    task_id: String,
    attempt_id: String,
    fence: u64,
    audit_intent: String,
    idempotency_key: String,
    request_fingerprint: String,
    artifacts: Vec<CommittedArtifact>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct VaultIdentity {
    format_version: u32,
    vault_uuid: VaultUuid,
}

impl VaultIdentity {
    fn decode(bytes: &[u8]) -> Result<VaultUuid, BrokerError> {
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
struct MutationReplay {
    fingerprint: String,
    expected_revision: Revision,
    receipt: Option<MutationReceipt>,
}

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

/// 基于文件系统的 canonical Vault 适配器。
pub struct FsVaultBroker {
    root: Dir,
    data: Dir,
    vault_uuid: VaultUuid,
    writer: writer::Writer,
    capture_instance: std::sync::Arc<()>,
    recovery_review: AtomicU64,
    history_pin: Mutex<Option<history::store::Pin>>,
    maintenance_pin: Mutex<Option<maintenance::store::Pin>>,
    file_operation_pin: std::sync::Mutex<Option<file_operation::binding::Pin>>,
    read_guard: Mutex<()>,
}

impl FsVaultBroker {
    /// 显式初始化或采用经过验证的旧存储；普通写者重开应使用 `open_existing`。
    pub fn open(vault_root: &Path, data_root: &Path) -> Result<Self, BrokerError> {
        Self::open_writer(vault_root, data_root, true)
    }

    fn fingerprint<T: Serialize>(value: &T) -> Result<String, BrokerError> {
        let bytes = serde_json::to_vec(value)
            .map_err(|error| BrokerError::json("serialize request fingerprint", error))?;
        Ok(sha256(&bytes))
    }

    fn inspect_existing(&self, path: &VaultPath) -> Result<Option<VaultObject>, BrokerError> {
        match self.read_checked(path, Some(writer::disk::CONTENT_LIMIT)) {
            Ok(object) => Ok(Some(object)),
            Err(BrokerError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn validate_components(
        &self,
        path: &VaultPath,
        allow_missing: bool,
    ) -> Result<(), BrokerError> {
        let components: Vec<&str> = path.as_str().split('/').collect();
        let mut current = PathBuf::new();
        for (index, component) in components.iter().enumerate() {
            let parent = current.clone();
            let requested_key = collision_component_key(component);
            let parent_path = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                &parent
            };
            let parent_dir = Self::open_directory(&self.root, parent_path, path)?;
            let listing = parent_dir.entries();
            match listing {
                Ok(entries) => {
                    for entry in entries {
                        let entry =
                            entry.map_err(|error| BrokerError::io("read directory", error))?;
                        let existing = entry.file_name();
                        let existing =
                            existing.to_str().ok_or_else(|| BrokerError::UnsafeNode {
                                path: path.clone(),
                                reason: "non-UTF-8 name",
                            })?;
                        if existing != *component
                            && collision_component_key(existing) == requested_key
                        {
                            let existing_path = parent.join(existing);
                            return Err(BrokerError::PathCollision {
                                requested: path.clone(),
                                existing: existing_path.to_string_lossy().into_owned(),
                            });
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && allow_missing => {
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(BrokerError::NotFound(path.clone()));
                }
                Err(error) => return Err(BrokerError::io("list path parent", error)),
            }
            current.push(component);
            match parent_dir.symlink_metadata(component) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        return Err(BrokerError::UnsafeNode {
                            path: path.clone(),
                            reason: "symbolic link",
                        });
                    }
                    let is_last = index + 1 == components.len();
                    if !is_last && !metadata.is_dir() {
                        return Err(BrokerError::UnsafeNode {
                            path: path.clone(),
                            reason: "non-directory path component",
                        });
                    }
                    if is_last && !metadata.is_file() {
                        return Err(BrokerError::UnsafeNode {
                            path: path.clone(),
                            reason: "not a regular file",
                        });
                    }
                    #[cfg(unix)]
                    if is_last && metadata.nlink() > 1 {
                        return Err(BrokerError::UnsafeNode {
                            path: path.clone(),
                            reason: "hard-linked file",
                        });
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && allow_missing => {
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(BrokerError::NotFound(path.clone()));
                }
                Err(error) => return Err(BrokerError::io("inspect Vault path", error)),
            }
        }
        Ok(())
    }

    fn ensure_parent_dirs(&self, path: &VaultPath) -> Result<(), BrokerError> {
        let mut current = PathBuf::new();
        let mut components = path.as_str().split('/').peekable();
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                break;
            }
            let parent_path = if current.as_os_str().is_empty() {
                Path::new(".")
            } else {
                &current
            };
            let parent_dir = Self::open_directory(&self.root, parent_path, path)?;
            match parent_dir.symlink_metadata(component) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(BrokerError::UnsafeNode {
                        path: path.clone(),
                        reason: "unsafe parent directory",
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    match parent_dir.create_dir(component) {
                        Ok(()) => Self::sync_directory(&parent_dir, Path::new("."))?,
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(BrokerError::io("create Vault directory", error)),
                    }
                }
                Err(error) => return Err(BrokerError::io("inspect Vault directory", error)),
            }
            current.push(component);
            // 创建后从已批准的根重新打开：并发的改名或链接替换，
            // 不得成为后续子目录的权威来源。
            Self::open_directory(&self.root, &current, path)?;
        }
        Ok(())
    }

    fn atomic_write_dir_checked(
        dir: &Dir,
        path: &Path,
        content: &[u8],
        create_new: bool,
        before_install: impl FnOnce() -> Result<(), BrokerError>,
    ) -> Result<(), BrokerError> {
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(BrokerError::CorruptIdentity)?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(
            ".{file_name}.studiolo-tmp-{}-{sequence}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut temp = dir
            .open_with(&temp_path, &options)
            .map_err(|error| BrokerError::io("create atomic temporary file", error))?;
        let write_result = (|| {
            temp.write_all(content)
                .map_err(|error| BrokerError::io("write atomic temporary file", error))?;
            temp.sync_all()
                .map_err(|error| BrokerError::io("sync atomic temporary file", error))?;
            before_install()?;
            if create_new {
                #[cfg(target_os = "linux")]
                renameat_with(dir, &temp_path, dir, path, RenameFlags::NOREPLACE)
                    .map_err(|error| BrokerError::io("install create-only file", error.into()))?;
                #[cfg(not(target_os = "linux"))]
                {
                    // link(2) 具备原子的仅创建语义；
                    // 先查元数据再用会覆盖目标的 rename(2) 则做不到。
                    dir.hard_link(&temp_path, dir, path)
                        .map_err(|error| BrokerError::io("install create-only file", error))?;
                    dir.remove_file(&temp_path).map_err(|error| {
                        BrokerError::io("unlink installed temporary file", error)
                    })?;
                }
            } else {
                dir.rename(&temp_path, dir, path)
                    .map_err(|error| BrokerError::io("rename atomic temporary file", error))?;
            }
            Self::sync_directory(dir, parent)
        })();
        if write_result.is_err() {
            let _ = dir.remove_file(&temp_path);
        }
        write_result
    }

    #[cfg(target_os = "linux")]
    fn sync_directory(dir: &Dir, path: &Path) -> Result<(), BrokerError> {
        // cap-std 可能返回无法 fsync 的 O_PATH 目录句柄；
        // 这里在保持同一有根解析的前提下另取一个可读 fd。
        let handle = openat2(
            dir,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|error| BrokerError::io("open parent for sync", error.into()))?;
        std::fs::File::from(handle)
            .sync_all()
            .map_err(|error| BrokerError::io("sync parent directory", error))
    }

    #[cfg(not(target_os = "linux"))]
    fn sync_directory(dir: &Dir, path: &Path) -> Result<(), BrokerError> {
        dir.open_dir(path)
            .map_err(|error| BrokerError::io("open parent for sync", error))?
            .into_std_file()
            .sync_all()
            .map_err(|error| BrokerError::io("sync parent directory", error))
    }

    #[cfg(target_os = "linux")]
    fn open_directory(root: &Dir, directory: &Path, path: &VaultPath) -> Result<Dir, BrokerError> {
        let handle = openat2(
            root,
            directory,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|_| BrokerError::UnsafeNode {
            path: path.clone(),
            reason: "parent failed Linux beneath/no-link resolution",
        })?;
        Ok(Dir::from_std_file(std::fs::File::from(handle)))
    }

    #[cfg(not(target_os = "linux"))]
    fn open_directory(root: &Dir, directory: &Path, _path: &VaultPath) -> Result<Dir, BrokerError> {
        root.open_dir(directory)
            .map_err(|error| BrokerError::io("open rooted directory", error))
    }

    #[cfg(target_os = "linux")]
    fn open_rooted_file(root: &Dir, path: &VaultPath) -> Result<cap_std::fs::File, BrokerError> {
        let handle = openat2(
            root,
            path.as_str(),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|error| {
            let error = std::io::Error::from(error);
            if error.kind() == std::io::ErrorKind::NotFound {
                BrokerError::NotFound(path.clone())
            } else {
                BrokerError::UnsafeNode {
                    path: path.clone(),
                    reason: "file failed Linux beneath/no-link resolution",
                }
            }
        })?;
        Ok(cap_std::fs::File::from_std(std::fs::File::from(handle)))
    }

    #[cfg(not(target_os = "linux"))]
    fn open_rooted_file(root: &Dir, path: &VaultPath) -> Result<cap_std::fs::File, BrokerError> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt as _;
            options.custom_flags(
                (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOFOLLOW).bits() as i32,
            );
        }
        root.open_with(path.as_str(), &options).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                BrokerError::NotFound(path.clone())
            } else {
                BrokerError::io("open Vault object", error)
            }
        })
    }

    fn validate_file_metadata(
        path: &VaultPath,
        metadata: &cap_std::fs::Metadata,
    ) -> Result<(), BrokerError> {
        if !metadata.is_file() {
            return Err(BrokerError::UnsafeNode {
                path: path.clone(),
                reason: "not a regular file",
            });
        }
        #[cfg(unix)]
        if metadata.nlink() != 1 {
            return Err(BrokerError::UnsafeNode {
                path: path.clone(),
                reason: "hard-linked or unlinked file",
            });
        }
        Ok(())
    }

    fn snapshot_storage_path(id: &SnapshotId) -> PathBuf {
        Path::new("snapshots").join(format!("{}.json", id.as_str()))
    }

    fn manifest_path(task_id: TaskId, attempt_id: AttemptId) -> Result<VaultPath, BrokerError> {
        VaultPath::parse(format!(
            "sources/attempts/{task_id}/{attempt_id}/manifest.json"
        ))
        .map_err(|_| BrokerError::InvalidArtifact("derived manifest path was invalid".to_owned()))
    }
}

impl VaultReader for FsVaultBroker {
    fn vault_uuid(&self) -> VaultUuid {
        self.vault_uuid
    }

    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError> {
        let committed = capture::ready(self)?;
        let object = self.read_checked(path, None)?;
        self.check_read_watermark(committed)?;
        Ok(object)
    }
}

impl FsVaultBroker {
    // 仅公开当前读取使用此门禁；安装与证据保全必须仍可私下观察未知对象。
    fn check_read_watermark(&self, committed: u64) -> Result<(), BrokerError> {
        if capture::ready(self)? != committed {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(())
    }

    fn read_checked(
        &self,
        path: &VaultPath,
        max_bytes: Option<usize>,
    ) -> Result<VaultObject, BrokerError> {
        self.writer.check(&self.root, &self.data)?;
        let _read_guard = self.read_guard.lock().map_err(|_| memory_state_error())?;
        self.validate_components(path, false)?;
        let mut file = Self::open_rooted_file(&self.root, path)?;
        let before = file
            .metadata()
            .map_err(|error| BrokerError::io("stat Vault object before read", error))?;
        Self::validate_file_metadata(path, &before)?;
        let mut content = Vec::new();
        if let Some(max) = max_bytes {
            if before.len() > max as u64 {
                return Err(BrokerError::WriterLimitExceeded);
            }
            (&mut file)
                .take(max as u64 + 1)
                .read_to_end(&mut content)
                .map_err(|error| BrokerError::io("read bounded Vault object", error))?;
            if content.len() > max {
                return Err(BrokerError::WriterLimitExceeded);
            }
        } else {
            file.read_to_end(&mut content)
                .map_err(|error| BrokerError::io("read Vault object", error))?;
        }
        let after = file
            .metadata()
            .map_err(|error| BrokerError::io("stat Vault object after read", error))?;
        Self::validate_file_metadata(path, &after)?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err(BrokerError::UnstableRead(path.clone()));
        }
        let current_file = Self::open_rooted_file(&self.root, path)?;
        let current = current_file
            .metadata()
            .map_err(|error| BrokerError::io("stat current Vault object", error))?;
        Self::validate_file_metadata(path, &current)?;
        #[cfg(unix)]
        if before.dev() != current.dev()
            || before.ino() != current.ino()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || after.ctime() != current.ctime()
            || after.ctime_nsec() != current.ctime_nsec()
        {
            return Err(BrokerError::UnstableRead(path.clone()));
        }
        if after.len() != current.len() || after.modified().ok() != current.modified().ok() {
            return Err(BrokerError::UnstableRead(path.clone()));
        }
        self.writer.check(&self.root, &self.data)?;
        Ok(VaultObject {
            revision: Revision::compute(self.vault_uuid, path, &content),
            path: path.clone(),
            content,
        })
    }
}

impl VaultSnapshotter for FsVaultBroker {
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError> {
        // 空输入也必须先过门禁，不能在未知安装期间发布一个成功快照。
        let committed = capture::ready(self)?;
        let mut inputs = inputs.to_vec();
        inputs.sort_by(|left, right| left.path.cmp(&right.path));
        if inputs.windows(2).any(|pair| pair[0].path == pair[1].path) {
            return Err(BrokerError::InvalidArtifact(
                "snapshot contains duplicate paths".to_owned(),
            ));
        }
        let mut files = Vec::with_capacity(inputs.len());
        for input in inputs {
            let object = self.read(&input.path)?;
            if object.revision != input.expected_revision {
                return Err(BrokerError::RevisionConflict {
                    path: input.path,
                    expected: input.expected_revision.to_string(),
                    actual: Some(object.revision),
                });
            }
            files.push(SnapshotFile {
                path: object.path,
                revision: object.revision,
                content: object.content,
            });
        }
        for captured in &files {
            if self.read(&captured.path)?.revision != captured.revision {
                return Err(BrokerError::UnstableRead(captured.path.clone()));
            }
        }
        let id = snapshot_identity(self.vault_uuid, &files)?;
        let snapshot = Snapshot {
            id: id.clone(),
            vault_uuid: self.vault_uuid,
            files,
        };
        let bytes = serde_json::to_vec(&snapshot)
            .map_err(|error| BrokerError::json("serialize snapshot", error))?;
        let storage_path = Self::snapshot_storage_path(&id);
        self.check_read_watermark(committed)?;
        match self.data.open(&storage_path) {
            Ok(_) => {
                if self.load_snapshot(&id)? != snapshot {
                    return Err(BrokerError::CorruptSnapshot);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Self::atomic_write_dir_checked(&self.data, &storage_path, &bytes, true, || {
                    self.check_read_watermark(committed)
                })?
            }
            Err(error) => return Err(BrokerError::io("open snapshot record", error)),
        }
        self.check_read_watermark(committed)?;
        Ok(snapshot)
    }
}

impl SnapshotReader for FsVaultBroker {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError> {
        let mut file = self
            .data
            .open(Self::snapshot_storage_path(id))
            .map_err(|error| BrokerError::io("open snapshot", error))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| BrokerError::io("read snapshot", error))?;
        let snapshot: Snapshot = serde_json::from_slice(&bytes)
            .map_err(|error| BrokerError::json("parse snapshot", error))?;
        if snapshot.id != *id
            || snapshot.vault_uuid != self.vault_uuid
            || snapshot
                .files
                .windows(2)
                .any(|pair| pair[0].path >= pair[1].path)
            || snapshot.files.iter().any(|file| {
                Revision::compute(self.vault_uuid, &file.path, &file.content) != file.revision
            })
            || snapshot_identity(self.vault_uuid, &snapshot.files)? != *id
        {
            return Err(BrokerError::CorruptSnapshot);
        }
        Ok(snapshot)
    }
}

impl VaultWriter for FsVaultBroker {
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError> {
        mutation::apply(self, mutation)
    }

    fn commit_attempt_artifacts(
        &mut self,
        mut commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError> {
        if self.write_status()?.unresolved {
            return Err(BrokerError::MutationUnresolved);
        }
        if commit.audit_intent.is_empty() || commit.idempotency_key.is_empty() {
            return Err(BrokerError::InvalidArtifact(
                "audit intent and idempotency key are required".to_owned(),
            ));
        }
        commit
            .artifacts
            .sort_by(|left, right| left.name.cmp(&right.name));
        let mut seen = BTreeSet::new();
        for artifact in &commit.artifacts {
            if artifact.name.as_str() == "manifest.json" || artifact.name.is_protected() {
                return Err(BrokerError::InvalidArtifact(format!(
                    "reserved artifact name: {}",
                    artifact.name
                )));
            }
            if !seen.insert(artifact.name.collision_key()) {
                return Err(BrokerError::InvalidArtifact(
                    "artifact names collide".to_owned(),
                ));
            }
        }
        let request_fingerprint = Self::fingerprint(&commit)?;
        let manifest_path = Self::manifest_path(commit.task_id, commit.attempt_id)?;
        if let Some(existing) = self.inspect_existing(&manifest_path)? {
            let manifest: AttemptManifest = serde_json::from_slice(&existing.content)
                .map_err(|error| BrokerError::json("parse attempt manifest", error))?;
            if manifest.idempotency_key == commit.idempotency_key
                && manifest.request_fingerprint == request_fingerprint
            {
                verify_manifest(self, &commit, &manifest, &existing)?;
                return Ok(ArtifactCommitReceipt {
                    manifest_path,
                    manifest_revision: existing.revision,
                    artifacts: manifest.artifacts,
                });
            }
            return Err(BrokerError::IdempotencyConflict);
        }

        let base = format!("sources/attempts/{}/{}", commit.task_id, commit.attempt_id);
        let mut artifacts = Vec::with_capacity(commit.artifacts.len());
        for candidate in &commit.artifacts {
            let target = VaultPath::parse(format!("{base}/{}", candidate.name.as_str()))
                .map_err(|_| BrokerError::InvalidArtifact("invalid derived target".to_owned()))?;
            let expected = Revision::compute(self.vault_uuid, &target, &candidate.content);
            let revision = match self.inspect_existing(&target)? {
                Some(existing) if existing.revision == expected => existing.revision,
                Some(_) => return Err(BrokerError::ArtifactConflict(target)),
                None => {
                    self.mutate(Mutation::create(
                        target.clone(),
                        candidate.content.clone(),
                        format!("artifact:{}:{}", commit.idempotency_key, candidate.name),
                    ))?
                    .revision
                }
            };
            artifacts.push(CommittedArtifact {
                path: target,
                revision,
                sha256: sha256(&candidate.content),
            });
        }

        let manifest = AttemptManifest {
            format_version: 1,
            task_id: commit.task_id.to_string(),
            attempt_id: commit.attempt_id.to_string(),
            fence: commit.fence.get(),
            audit_intent: commit.audit_intent,
            idempotency_key: commit.idempotency_key.clone(),
            request_fingerprint,
            artifacts: artifacts.clone(),
        };
        verify_artifact_records(self, &artifacts)?;
        let bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| BrokerError::json("serialize attempt manifest", error))?;
        let receipt = self.mutate(Mutation::create(
            manifest_path,
            bytes,
            format!("manifest:{}", commit.idempotency_key),
        ))?;
        Ok(ArtifactCommitReceipt {
            manifest_path: receipt.path,
            manifest_revision: receipt.revision,
            artifacts,
        })
    }
}

/// 内存契约适配器，遵循相同的逻辑 CAS 与 manifest 规则。
pub struct MemoryVaultBroker {
    vault_uuid: VaultUuid,
    objects: std::collections::BTreeMap<VaultPath, Vec<u8>>,
    history: std::collections::BTreeMap<Revision, Vec<u8>>,
    snapshots: Mutex<std::collections::BTreeMap<SnapshotId, Snapshot>>,
    replays: std::collections::BTreeMap<String, (String, mutation::record::Outcome)>,
    artifact_stages: Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
    batch_replays: std::collections::BTreeMap<String, batch::memory::MemoryBatch>,
    gateway_seals: std::collections::BTreeMap<VaultPath, Vec<u8>>,
}

impl Default for MemoryVaultBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryVaultBroker {
    /// 用新生成的 Vault 身份创建空适配器。
    #[must_use]
    pub fn new() -> Self {
        Self::with_uuid(VaultUuid::new())
    }

    /// 用指定的 Vault 持久身份创建空适配器。
    #[must_use]
    pub fn with_uuid(vault_uuid: VaultUuid) -> Self {
        Self {
            vault_uuid,
            objects: std::collections::BTreeMap::new(),
            history: std::collections::BTreeMap::new(),
            snapshots: Mutex::new(std::collections::BTreeMap::new()),
            replays: std::collections::BTreeMap::new(),
            artifact_stages: Mutex::new(std::collections::BTreeMap::new()),
            batch_replays: std::collections::BTreeMap::new(),
            gateway_seals: std::collections::BTreeMap::new(),
        }
    }

    /// 返回 L0 历史对象，供契约断言使用。
    #[must_use]
    pub fn history_object(&self, revision: Revision) -> Option<&[u8]> {
        self.history.get(&revision).map(Vec::as_slice)
    }

    fn memory_snapshot_id(&self, files: &[SnapshotFile]) -> Result<SnapshotId, BrokerError> {
        snapshot_identity(self.vault_uuid, files)
    }

    /// 与 `FsVaultBroker::inspect_existing` 同形：缺失是 `None`，其余失败一律上抛，
    /// 不把 `PathCollision` / `UnsafeNode` 静默当成「还不存在」。
    fn inspect_existing(&self, path: &VaultPath) -> Result<Option<VaultObject>, BrokerError> {
        match self.read(path) {
            Ok(object) => Ok(Some(object)),
            Err(BrokerError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl VaultReader for MemoryVaultBroker {
    fn vault_uuid(&self) -> VaultUuid {
        self.vault_uuid
    }

    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError> {
        memory_namespace::guard_read(self, path)?;
        let content = if path.as_str() == memory_namespace::IDENTITY_MARKER {
            memory_namespace::identity_bytes(self.vault_uuid)?
        } else {
            self.objects
                .get(path)
                .cloned()
                .ok_or_else(|| BrokerError::NotFound(path.clone()))?
        };
        Ok(VaultObject {
            revision: Revision::compute(self.vault_uuid, path, &content),
            path: path.clone(),
            content,
        })
    }
}

impl VaultSnapshotter for MemoryVaultBroker {
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError> {
        let mut inputs = inputs.to_vec();
        inputs.sort_by(|left, right| left.path.cmp(&right.path));
        if inputs.windows(2).any(|pair| pair[0].path == pair[1].path) {
            return Err(BrokerError::InvalidArtifact(
                "snapshot contains duplicate paths".to_owned(),
            ));
        }
        let mut files = Vec::with_capacity(inputs.len());
        for input in inputs {
            let object = self.read(&input.path)?;
            if object.revision != input.expected_revision {
                return Err(BrokerError::RevisionConflict {
                    path: input.path,
                    expected: input.expected_revision.to_string(),
                    actual: Some(object.revision),
                });
            }
            files.push(SnapshotFile {
                path: object.path,
                revision: object.revision,
                content: object.content,
            });
        }
        let snapshot = Snapshot {
            id: self.memory_snapshot_id(&files)?,
            vault_uuid: self.vault_uuid,
            files,
        };
        self.snapshots
            .lock()
            .map_err(|_| memory_state_error())?
            .insert(snapshot.id.clone(), snapshot.clone());
        Ok(snapshot)
    }
}

impl SnapshotReader for MemoryVaultBroker {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError> {
        self.snapshots
            .lock()
            .map_err(|_| memory_state_error())?
            .get(id)
            .cloned()
            .ok_or_else(|| {
                BrokerError::io(
                    "load memory snapshot",
                    std::io::Error::from(std::io::ErrorKind::NotFound),
                )
            })
    }
}

impl VaultWriter for MemoryVaultBroker {
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError> {
        memory_mutation::apply(self, mutation)
    }

    fn commit_attempt_artifacts(
        &mut self,
        mut commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError> {
        if commit.audit_intent.is_empty() || commit.idempotency_key.is_empty() {
            return Err(BrokerError::InvalidArtifact(
                "audit intent and idempotency key are required".to_owned(),
            ));
        }
        commit
            .artifacts
            .sort_by(|left, right| left.name.cmp(&right.name));
        // 与磁盘同序：保留名与撞名必须在任何产物落盘之前判完，被拒绝的请求零副作用。
        let mut seen = BTreeSet::new();
        for artifact in &commit.artifacts {
            if artifact.name.as_str() == "manifest.json" || artifact.name.is_protected() {
                return Err(BrokerError::InvalidArtifact(format!(
                    "reserved artifact name: {}",
                    artifact.name
                )));
            }
            if !seen.insert(artifact.name.collision_key()) {
                return Err(BrokerError::InvalidArtifact(
                    "artifact names collide".to_owned(),
                ));
            }
        }
        let request_fingerprint = FsVaultBroker::fingerprint(&commit)?;
        let manifest_path = FsVaultBroker::manifest_path(commit.task_id, commit.attempt_id)?;
        if let Some(existing) = self.inspect_existing(&manifest_path)? {
            let manifest: AttemptManifest = serde_json::from_slice(&existing.content)
                .map_err(|error| BrokerError::json("parse attempt manifest", error))?;
            if manifest.idempotency_key != commit.idempotency_key
                || manifest.request_fingerprint != request_fingerprint
            {
                return Err(BrokerError::IdempotencyConflict);
            }
            verify_manifest(self, &commit, &manifest, &existing)?;
            return Ok(ArtifactCommitReceipt {
                manifest_path,
                manifest_revision: existing.revision,
                artifacts: manifest.artifacts,
            });
        }
        let base = format!("sources/attempts/{}/{}", commit.task_id, commit.attempt_id);
        let mut artifacts = Vec::new();
        for candidate in &commit.artifacts {
            let path = VaultPath::parse(format!("{base}/{}", candidate.name.as_str()))
                .map_err(|_| BrokerError::InvalidArtifact("invalid derived target".to_owned()))?;
            let expected = Revision::compute(self.vault_uuid, &path, &candidate.content);
            let revision = match self.inspect_existing(&path)? {
                Some(object) if object.revision == expected => object.revision,
                Some(_) => return Err(BrokerError::ArtifactConflict(path)),
                None => {
                    self.mutate(Mutation::create(
                        path.clone(),
                        candidate.content.clone(),
                        format!("artifact:{}:{}", commit.idempotency_key, candidate.name),
                    ))?
                    .revision
                }
            };
            artifacts.push(CommittedArtifact {
                path,
                revision,
                sha256: sha256(&candidate.content),
            });
        }
        let manifest = AttemptManifest {
            format_version: 1,
            task_id: commit.task_id.to_string(),
            attempt_id: commit.attempt_id.to_string(),
            fence: commit.fence.get(),
            audit_intent: commit.audit_intent,
            idempotency_key: commit.idempotency_key.clone(),
            request_fingerprint,
            artifacts: artifacts.clone(),
        };
        verify_artifact_records(self, &artifacts)?;
        let bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| BrokerError::json("serialize attempt manifest", error))?;
        let manifest_receipt = self.mutate(Mutation::create(
            manifest_path,
            bytes,
            format!("manifest:{}", commit.idempotency_key),
        ))?;
        Ok(ArtifactCommitReceipt {
            manifest_path: manifest_receipt.path,
            manifest_revision: manifest_receipt.revision,
            artifacts,
        })
    }
}

fn snapshot_identity(
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

fn verify_manifest(
    broker: &impl VaultReader,
    commit: &ArtifactCommit,
    manifest: &AttemptManifest,
    existing: &VaultObject,
) -> Result<(), BrokerError> {
    if manifest.format_version != 1
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

fn verify_artifact_records(
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

fn memory_state_error() -> BrokerError {
    BrokerError::io(
        "access memory adapter state",
        std::io::Error::other("memory adapter mutex poisoned"),
    )
}

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

/// 磁盘条目名先归一到 NFC 再折叠大小写：VaultPath 只接受 NFC，而外部写者可能留下 NFD 拼写，
/// 在归一化不敏感的卷上两者是同一物理文件。
fn collision_component_key(component: &str) -> String {
    use unicode_normalization::UnicodeNormalization as _;
    component.nfc().collect::<String>().to_lowercase()
}
