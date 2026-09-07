//! Capability-rooted access to the canonical Studiolo Vault.
//!
//! This crate is the only production crate allowed to mutate canonical Vault
//! files. Callers provide logical [`VaultPath`] values, never ambient paths.

mod canonical;
mod capture;
mod mutation;
mod readonly;
mod writer;

pub use canonical::{CanonicalRead, CanonicalReadVault};
pub use capture::{CanonicalCapture, CaptureStamp};
pub use readonly::{ReadBatch, ReadEntry, ReadEntryKind, ReadNote, ReadOnlyVault, ScanLimits};
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

/// Failure modes exposed at the Vault boundary.
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
    /// 写者协议触及的输入或持久记录超过固定预算。
    #[error("writer storage or input limit exceeded")]
    WriterLimitExceeded,
    /// 捕获所属实例或已确认提交水位变化，候选视图不能发布。
    #[error("canonical capture changed")]
    CaptureChanged,
    /// 扫描出现安全诊断，不能把部分结果当作完整捕获。
    #[error("canonical capture is incomplete")]
    IncompleteCapture,
    /// A protected control path was sent through the ordinary mutation API.
    #[error("protected Vault path: {0}")]
    ProtectedPath(VaultPath),
    /// The requested object does not exist.
    #[error("Vault object not found: {0}")]
    NotFound(VaultPath),
    /// A compare-and-swap precondition did not match.
    #[error("revision conflict at {path}: expected {expected}, found {actual:?}")]
    RevisionConflict {
        /// Logical path that conflicted.
        path: VaultPath,
        /// Required revision, or absence when creating.
        expected: String,
        /// Revision observed by the Broker.
        actual: Option<Revision>,
    },
    /// A path component was a symlink, hardlink, or special file.
    #[error("unsafe filesystem node at {path}: {reason}")]
    UnsafeNode {
        /// Logical path rejected.
        path: VaultPath,
        /// Non-sensitive reason.
        reason: &'static str,
    },
    /// A spelling would alias an existing name on a common filesystem.
    #[error("path collision between {requested} and existing {existing}")]
    PathCollision {
        /// Requested logical path.
        requested: VaultPath,
        /// Existing logical spelling.
        existing: String,
    },
    /// An idempotency key was reused for different input.
    #[error("idempotency key was reused with different input")]
    IdempotencyConflict,
    /// An artifact target or commit request was invalid.
    #[error("invalid artifact commit: {0}")]
    InvalidArtifact(String),
    /// An orphan artifact conflicts with a retry of the same commit.
    #[error("artifact already exists with different content: {0}")]
    ArtifactConflict(VaultPath),
    /// The persisted Vault identity was malformed.
    #[error("corrupt Vault identity")]
    CorruptIdentity,
    /// The approved canonical root no longer resolves to its pinned directory.
    #[error("canonical Vault root changed or is unavailable")]
    RootChanged,
    /// A bounded read exceeded its caller-supplied byte budget.
    #[error("read limit of {max_bytes} bytes exceeded at {path}")]
    ReadLimitExceeded {
        /// Logical path whose bytes were refused.
        path: VaultPath,
        /// Maximum permitted content bytes, including growth during the read.
        max_bytes: u64,
    },
    /// Stored snapshot metadata or content failed integrity verification.
    #[error("corrupt snapshot record")]
    CorruptSnapshot,
    /// A snapshot changed while it was being captured.
    #[error("source changed while snapshotting: {0}")]
    UnstableRead(VaultPath),
    /// An operating-system operation failed without exposing an ambient path.
    #[error("Vault I/O failed during {operation}: {source}")]
    Io {
        /// Stable operation label.
        operation: &'static str,
        /// Underlying operating-system error.
        #[source]
        source: std::io::Error,
    },
    /// A durable record was not valid JSON.
    #[error("invalid durable record during {operation}: {source}")]
    Json {
        /// Stable operation label.
        operation: &'static str,
        /// JSON error.
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

/// Compare-and-swap condition for one mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precondition {
    /// The path must not exist.
    MustNotExist,
    /// The current content must have this revision.
    Match(Revision),
}

/// One ordinary canonical Vault mutation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mutation {
    /// Logical target path.
    pub path: VaultPath,
    /// Complete replacement content.
    pub content: Vec<u8>,
    /// Required prior state.
    pub precondition: Precondition,
    /// Stable caller-supplied replay key.
    pub idempotency_key: String,
}

impl Mutation {
    /// Constructs a create-only mutation.
    #[must_use]
    pub fn create(path: VaultPath, content: Vec<u8>, idempotency_key: impl Into<String>) -> Self {
        Self {
            path,
            content,
            precondition: Precondition::MustNotExist,
            idempotency_key: idempotency_key.into(),
        }
    }

    /// Constructs a compare-and-swap replacement.
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

/// Canonical object returned from a stable read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultObject {
    /// Logical path.
    pub path: VaultPath,
    /// Complete content.
    pub content: Vec<u8>,
    /// Revision bound to Vault identity, path, and content.
    pub revision: Revision,
}

/// Durable result of one mutation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationReceipt {
    /// Mutated logical path.
    pub path: VaultPath,
    /// Revision after the write.
    pub revision: Revision,
    /// Revision stored in L0 history, if a prior object existed.
    pub previous_revision: Option<Revision>,
}

/// A path and exact revision requested for a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotInput {
    /// Logical source path.
    pub path: VaultPath,
    /// Revision that must still be current.
    pub expected_revision: Revision,
}

/// Content-addressed snapshot identifier.
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
    /// Returns the hexadecimal identifier.
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

/// One immutable file in a runner-facing snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFile {
    /// Logical Vault path; no ambient host path is exposed.
    pub path: VaultPath,
    /// Captured revision.
    pub revision: Revision,
    /// Captured bytes.
    pub content: Vec<u8>,
}

/// Immutable, durable input snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Deterministic identifier derived from its manifest.
    pub id: SnapshotId,
    /// Vault identity at capture time.
    pub vault_uuid: VaultUuid,
    /// Files sorted by logical path.
    pub files: Vec<SnapshotFile>,
}

/// One runner-produced artifact candidate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCandidate {
    /// Relative name underneath the attempt directory.
    pub name: VaultPath,
    /// Candidate bytes copied from runner-owned temporary storage.
    pub content: Vec<u8>,
}

/// Request to publish one attempt's artifacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCommit {
    /// Owning task.
    pub task_id: TaskId,
    /// Owning attempt.
    pub attempt_id: AttemptId,
    /// Current attempt fence.
    pub fence: Fence,
    /// Identifier of the already-appended audit intent.
    pub audit_intent: String,
    /// Stable replay key.
    pub idempotency_key: String,
    /// Candidate artifacts. The Broker derives all canonical destinations.
    pub artifacts: Vec<ArtifactCandidate>,
}

/// One artifact recorded in the attempt manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedArtifact {
    /// Canonical Vault path.
    pub path: VaultPath,
    /// Bound Vault revision.
    pub revision: Revision,
    /// Plain SHA-256 content digest.
    pub sha256: String,
}

/// Durable result of a manifest-last artifact commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCommitReceipt {
    /// Manifest commit point.
    pub manifest_path: VaultPath,
    /// Revision of the manifest itself.
    pub manifest_revision: Revision,
    /// Published artifacts.
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

/// Read capability for canonical Vault objects.
pub trait VaultReader: Send + Sync {
    /// Returns the persistent Vault identity.
    fn vault_uuid(&self) -> VaultUuid;

    /// Reads a regular, non-linked object and verifies a stable revision.
    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError>;
}

/// Snapshot creation capability.
pub trait VaultSnapshotter: Send + Sync {
    /// Captures exact revisions into an immutable, content-addressed snapshot.
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError>;
}

/// Read capability for previously captured snapshots.
pub trait SnapshotReader: Send + Sync {
    /// Loads a snapshot without exposing ambient host paths.
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError>;
}

/// The only ordinary capability that mutates canonical Vault files.
pub trait VaultWriter: Send + Sync {
    /// Performs one idempotent, CAS-guarded full-content mutation.
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError>;

    /// Publishes attempt artifacts and writes `manifest.json` last.
    fn commit_attempt_artifacts(
        &mut self,
        commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError>;
}

/// Complete Broker interface used by the composition root.
pub trait VaultBroker: VaultReader + VaultSnapshotter + SnapshotReader + VaultWriter {}

impl<T> VaultBroker for T where T: VaultReader + VaultSnapshotter + SnapshotReader + VaultWriter {}

/// Filesystem-backed canonical Vault adapter.
pub struct FsVaultBroker {
    root: Dir,
    data: Dir,
    vault_uuid: VaultUuid,
    writer: writer::Writer,
    capture_instance: std::sync::Arc<()>,
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
            // Reopen from the approved root after creation. A concurrent rename
            // or link replacement must not become authority for later children.
            Self::open_directory(&self.root, &current, path)?;
        }
        Ok(())
    }

    fn atomic_write_dir(
        dir: &Dir,
        path: &Path,
        content: &[u8],
        create_new: bool,
    ) -> Result<(), BrokerError> {
        Self::atomic_write_dir_checked(dir, path, content, create_new, || Ok(()))
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
                    // link(2) has atomic create-only semantics, unlike a
                    // metadata check followed by replacing rename(2).
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
        // cap-std may return an O_PATH directory, which cannot be fsynced.
        // Obtain a readable fd while retaining the same rooted resolution.
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
        self.read_checked(path, None)
    }
}

impl FsVaultBroker {
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
        match self.data.open(&storage_path) {
            Ok(_) => {
                if self.load_snapshot(&id)? != snapshot {
                    return Err(BrokerError::CorruptSnapshot);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Self::atomic_write_dir(&self.data, &storage_path, &bytes, true)?
            }
            Err(error) => return Err(BrokerError::io("open snapshot record", error)),
        }
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

/// In-memory contract adapter with the same logical CAS and manifest rules.
pub struct MemoryVaultBroker {
    vault_uuid: VaultUuid,
    objects: std::collections::BTreeMap<VaultPath, Vec<u8>>,
    history: std::collections::BTreeMap<Revision, Vec<u8>>,
    snapshots: Mutex<std::collections::BTreeMap<SnapshotId, Snapshot>>,
    replays: std::collections::BTreeMap<String, (String, MutationReceipt)>,
}

impl Default for MemoryVaultBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryVaultBroker {
    /// Creates an empty adapter with a fresh Vault identity.
    #[must_use]
    pub fn new() -> Self {
        Self::with_uuid(VaultUuid::new())
    }

    /// Creates an empty adapter with a specified durable Vault identity.
    #[must_use]
    pub fn with_uuid(vault_uuid: VaultUuid) -> Self {
        Self {
            vault_uuid,
            objects: std::collections::BTreeMap::new(),
            history: std::collections::BTreeMap::new(),
            snapshots: Mutex::new(std::collections::BTreeMap::new()),
            replays: std::collections::BTreeMap::new(),
        }
    }

    /// Returns an L0 history object for contract assertions.
    #[must_use]
    pub fn history_object(&self, revision: Revision) -> Option<&[u8]> {
        self.history.get(&revision).map(Vec::as_slice)
    }

    fn memory_snapshot_id(&self, files: &[SnapshotFile]) -> Result<SnapshotId, BrokerError> {
        snapshot_identity(self.vault_uuid, files)
    }
}

impl VaultReader for MemoryVaultBroker {
    fn vault_uuid(&self) -> VaultUuid {
        self.vault_uuid
    }

    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError> {
        let content = self
            .objects
            .get(path)
            .cloned()
            .ok_or_else(|| BrokerError::NotFound(path.clone()))?;
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
        if mutation.path.is_protected() {
            return Err(BrokerError::ProtectedPath(mutation.path));
        }
        let fingerprint = FsVaultBroker::fingerprint(&mutation)?;
        if let Some((prior_fingerprint, receipt)) = self.replays.get(&mutation.idempotency_key) {
            return if prior_fingerprint == &fingerprint {
                Ok(receipt.clone())
            } else {
                Err(BrokerError::IdempotencyConflict)
            };
        }
        if mutation.idempotency_key.is_empty() {
            return Err(BrokerError::IdempotencyConflict);
        }
        if let Some(existing) = self.objects.keys().find(|existing| {
            *existing != &mutation.path && spelling_collision(existing, &mutation.path)
        }) {
            return Err(BrokerError::PathCollision {
                requested: mutation.path,
                existing: existing.to_string(),
            });
        }
        let current = self.objects.get(&mutation.path).map(|content| VaultObject {
            path: mutation.path.clone(),
            revision: Revision::compute(self.vault_uuid, &mutation.path, content),
            content: content.clone(),
        });
        match (mutation.precondition, current.as_ref()) {
            (Precondition::MustNotExist, None) => {}
            (Precondition::MustNotExist, Some(object)) => {
                return Err(BrokerError::RevisionConflict {
                    path: mutation.path,
                    expected: "absence".to_owned(),
                    actual: Some(object.revision),
                });
            }
            (Precondition::Match(expected), Some(object)) if object.revision == expected => {}
            (Precondition::Match(expected), object) => {
                return Err(BrokerError::RevisionConflict {
                    path: mutation.path,
                    expected: expected.to_string(),
                    actual: object.map(|value| value.revision),
                });
            }
        }
        if let Some(object) = &current {
            self.history
                .entry(object.revision)
                .or_insert_with(|| object.content.clone());
        }
        let revision = Revision::compute(self.vault_uuid, &mutation.path, &mutation.content);
        self.objects.insert(mutation.path.clone(), mutation.content);
        let receipt = MutationReceipt {
            path: mutation.path,
            revision,
            previous_revision: current.map(|object| object.revision),
        };
        self.replays
            .insert(mutation.idempotency_key, (fingerprint, receipt.clone()));
        Ok(receipt)
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
        let request_fingerprint = FsVaultBroker::fingerprint(&commit)?;
        let manifest_path = FsVaultBroker::manifest_path(commit.task_id, commit.attempt_id)?;
        if let Ok(existing) = self.read(&manifest_path) {
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
        let mut seen = BTreeSet::new();
        let base = format!("sources/attempts/{}/{}", commit.task_id, commit.attempt_id);
        let mut artifacts = Vec::new();
        for candidate in &commit.artifacts {
            if candidate.name.as_str() == "manifest.json"
                || candidate.name.is_protected()
                || !seen.insert(candidate.name.collision_key())
            {
                return Err(BrokerError::InvalidArtifact(
                    "reserved or colliding artifact name".to_owned(),
                ));
            }
            let path = VaultPath::parse(format!("{base}/{}", candidate.name.as_str()))
                .map_err(|_| BrokerError::InvalidArtifact("invalid artifact target".to_owned()))?;
            let expected = Revision::compute(self.vault_uuid, &path, &candidate.content);
            let revision = match self.read(&path) {
                Ok(object) if object.revision == expected => object.revision,
                Ok(_) => return Err(BrokerError::ArtifactConflict(path)),
                Err(BrokerError::NotFound(_)) => {
                    self.mutate(Mutation::create(
                        path.clone(),
                        candidate.content.clone(),
                        format!("artifact:{}:{}", commit.idempotency_key, candidate.name),
                    ))?
                    .revision
                }
                Err(error) => return Err(error),
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

fn spelling_collision(existing: &VaultPath, requested: &VaultPath) -> bool {
    for (existing, requested) in existing
        .as_str()
        .split('/')
        .zip(requested.as_str().split('/'))
    {
        if collision_component_key(existing) != collision_component_key(requested) {
            return false;
        }
        if existing != requested {
            return true;
        }
    }
    false
}
