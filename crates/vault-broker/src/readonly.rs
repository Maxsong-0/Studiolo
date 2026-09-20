//! Zero-write access to an explicitly selected local replica or asset directory.

mod directory;
mod metadata;
mod private;
mod read;
mod rooted;
mod scan;
pub(crate) mod sync;
pub use sync::{LocalReadFile, LocalReadObservation, LocalReadPresence, LocalReadSurvey};

use std::{path::Path, sync::Arc};

use cap_std::fs::Dir;
use studiolo_core::{Revision, VaultPath, VaultUuid};

use crate::BrokerError;

/// One bounded, stable read from an explicitly selected directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadNote {
    /// Logical path relative to the selected root.
    pub path: VaultPath,
    /// Exact bytes; callers decide whether UTF-8 is required.
    pub content: Vec<u8>,
    /// Path/content fingerprint in a fixed read-only namespace, not Vault identity.
    pub revision: Revision,
}

/// 一次扫描的文件树与内存 Markdown；存在 diagnostics 即代表批次不完整。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadBatch {
    /// 目录、Markdown 与附件元信息，按逻辑路径排序；不含虚拟根目录。
    pub entries: Vec<ReadEntry>,
    /// Successfully read Markdown files in logical path order.
    pub notes: Vec<ReadNote>,
    /// Logical paths and fixed reasons only; never source bytes or host paths.
    pub diagnostics: Vec<String>,
}

/// 同一次只读扫描得到的可见条目；不含绝对路径、附件内容或伪造的内容 revision。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadEntry {
    /// 相对于显式选定根的规范逻辑路径。
    pub path: VaultPath,
    /// 安全检查后的条目类型。
    pub kind: ReadEntryKind,
    /// 文件的字节长度；目录为 None，不使用平台相关目录 inode 长度。
    pub size_bytes: Option<u64>,
}

/// 文件树条目类型；附件仅有元信息，不提供下载或预览能力。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadEntryKind {
    /// 已安全打开并遍历的目录，包括空目录与纯附件目录。
    Directory,
    /// 已成功稳定读取内容的 Markdown 文档。
    Markdown,
    /// 仅检查元信息的普通文件，不读取其字节。
    Attachment,
}

/// Resource budgets for one complete recursive scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanLimits {
    /// Maximum bytes retained from one Markdown file; oversized Markdown is diagnostic.
    pub max_file_bytes: u64,
    /// Maximum aggregate Markdown bytes; exceeding it fails the whole scan.
    pub max_total_bytes: u64,
    /// Maximum entries inspected, including hidden entries, directories and non-Markdown.
    pub max_entries: usize,
    /// Maximum nested directory depth; files directly under the root have depth zero.
    pub max_depth: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 4 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_entries: 50_000,
            max_depth: 32,
        }
    }
}

/// Directory-handle-rooted reader with no mutation, identity or database capability.
///
/// Unix opens every component without following links. Linux additionally uses
/// kernel beneath/no-link resolution. Unsupported platforms fail closed. Reads
/// may update operating-system access times, but never write source content or
/// create control files, caches, directories, snapshots or persistent identities.
pub struct ReadOnlyVault {
    root: Dir,
    private: bool,
    instance: Arc<()>,
}

impl ReadOnlyVault {
    pub(crate) fn check_batch_barrier(&self) -> Result<(), BrokerError> {
        if crate::batch::barrier::blocked(&self.root)? {
            return Err(BrokerError::MutationUnresolved);
        }
        Ok(())
    }
    /// Pins an existing directory, rejecting a symlink at the selected root.
    /// No directory is created, including when opening fails.
    pub fn open(root: &Path) -> Result<Self, BrokerError> {
        Ok(Self {
            root: rooted::open_root(root)?,
            private: false,
            instance: Arc::new(()),
        })
    }

    /// 打开私有只读根：绝对路径逐分量拒绝链接，祖先属主仅允许 root 或当前用户。
    /// 选定根及后续访问的目录须为当前用户的 0700，文件须为 0600；不创建任何内容。
    pub fn open_private(root: &Path) -> Result<Self, BrokerError> {
        Ok(Self {
            root: rooted::open_private_root(root)?,
            private: true,
            instance: Arc::new(()),
        })
    }

    /// 从已固定根句柄打开可见子目录，继承私有模式；不重新解析宿主绝对路径。
    pub fn open_directory(&self, path: &VaultPath) -> Result<Self, BrokerError> {
        visible(path)?;
        Ok(Self {
            root: rooted::open_directory(&self.root, path, self.private)?,
            private: self.private,
            instance: Arc::new(()),
        })
    }

    /// 返回当前已固定根句柄的设备/inode身份，用于与可信启动检查对象比较。
    /// 不是Vault持久身份，不读路径、不创建标记；不应暴露给HTTP调用者。
    #[cfg(unix)]
    pub fn root_identity(&self) -> Result<(u64, u64), BrokerError> {
        use cap_std::fs::MetadataExt as _;
        let metadata = self
            .root
            .dir_metadata()
            .map_err(|error| BrokerError::io("检查只读根句柄", error))?;
        Ok((metadata.dev(), metadata.ino()))
    }

    /// Reads a visible regular file, rejecting links, collisions and concurrent changes.
    /// The limit also applies if the file grows after it is opened. Asset files
    /// such as JavaScript and CSS are supported; hidden components are not.
    /// Each parent name check is capped at 50,000 entries, including hidden names.
    pub fn read_file(&self, path: &VaultPath, max_bytes: u64) -> Result<ReadNote, BrokerError> {
        self.read_in_namespace(path, max_bytes, read::replica_namespace())
    }

    pub(super) fn read_in_namespace(
        &self,
        path: &VaultPath,
        max_bytes: u64,
        namespace: VaultUuid,
    ) -> Result<ReadNote, BrokerError> {
        visible(path)?;
        read::read_with(path, max_bytes, namespace, || {
            rooted::open_file(&self.root, path, self.private)
        })
    }

    // The only hidden-path read is this fixed, bounded identity record.
    pub(super) fn read_identity(&self) -> Result<Vec<u8>, BrokerError> {
        let path =
            VaultPath::parse(".workbench/vault.json").map_err(|_| BrokerError::CorruptIdentity)?;
        let note = read::read_with(&path, 4096, read::replica_namespace(), || {
            rooted::open_file(&self.root, &path, self.private)
        })?;
        Ok(note.content)
    }

    fn read_scanned(
        &self,
        parent: &Dir,
        name: &str,
        path: &VaultPath,
        max_bytes: u64,
        namespace: VaultUuid,
    ) -> Result<ReadNote, BrokerError> {
        read::read_with(path, max_bytes, namespace, || {
            rooted::scanned_file(&self.root, parent, name, path, self.private)
        })
    }
}

// 仅供已持有 canonical 租约的 Broker 复用读取内核；临时句柄不逃逸。
pub(super) fn scan_bound(
    root: &Dir,
    limits: ScanLimits,
    namespace: VaultUuid,
) -> Result<ReadBatch, BrokerError> {
    bound(root)?.scan_in_namespace(limits, namespace)
}

pub(super) fn read_bound(
    root: &Dir,
    path: &VaultPath,
    max_bytes: u64,
    namespace: VaultUuid,
) -> Result<ReadNote, BrokerError> {
    bound(root)?.read_in_namespace(path, max_bytes, namespace)
}

fn bound(root: &Dir) -> Result<ReadOnlyVault, BrokerError> {
    Ok(ReadOnlyVault {
        root: root
            .try_clone()
            .map_err(|e| BrokerError::io("retain canonical read root", e))?,
        private: false,
        instance: Arc::new(()),
    })
}

fn visible(path: &VaultPath) -> Result<(), BrokerError> {
    if path.as_str().split('/').any(|part| part.starts_with('.')) {
        return Err(unsafe_node(path, "hidden path is not readable"));
    }
    Ok(())
}

fn unsafe_node(path: &VaultPath, reason: &'static str) -> BrokerError {
    BrokerError::UnsafeNode {
        path: path.clone(),
        reason,
    }
}

fn limit(reason: &'static str) -> BrokerError {
    BrokerError::io(reason, std::io::Error::other(reason))
}
