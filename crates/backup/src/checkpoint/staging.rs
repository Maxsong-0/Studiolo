//! 隔离 owner 导出、按真实字节过滤、视图和清单一次原子发布。
mod catalog;
mod creation;
mod inventory;
mod io;
mod lifecycle;
mod seal;
mod shared;
mod tree;

pub use catalog::{ObjectRegistration, RedactedObject, SourceCatalog};
pub use inventory::{SealedFile, SealedInventory};
use io::Directory;
pub use seal::{SealInputs, SealedCheckpoint, SealedReference};
pub use shared::ReaderIdentity;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use studiolo_core::checkpoint::CheckpointId;

/// 所有 owner 的固定导出位置；不能通过请求增加一个任意宿主机路径。
pub const OWNER_DIRECTORIES: [&str; 11] = [
    "canonical",
    "writer-data",
    "control",
    "auth",
    "bridge",
    "transport",
    "plane",
    "agent",
    "policy",
    "audit",
    "agent-audit",
];
const MAX_NODES: u64 = 1_000_000;
const MAX_BYTES: u64 = 512 * 1024 * 1024 * 1024;
const MAX_METADATA: u64 = 128 * 1024 * 1024;

/// 一次尝试独占的新目录；回收仅限这次新建目录的原身份，不覆盖其他尝试。
pub struct StagingArea {
    lifecycle: lifecycle::Lifecycle,
    parent: Directory,
    exports: Directory,
    pending: Directory,
    data: Directory,
    parent_path: PathBuf,
    export_path: PathBuf,
    export_name: String,
    pending_name: String,
    final_name: String,
    checkpoint_id: CheckpointId,
    protected: Vec<(PathBuf, Directory)>,
    ephemeral: (PathBuf, Directory),
    deadline: Instant,
    reader: Option<ReaderIdentity>,
}
impl StagingArea {
    /// 所有目录必须已获部署批准。ephemeral 与源/封存根须物理分离；链接不参与解析。
    /// 同一 checkpoint 同时只创建一次；原始导出在发布前删除，失败/展开也回收本次目录。
    pub fn create(
        parent: &Path,
        checkpoint_id: CheckpointId,
        source_roots: &[PathBuf],
        ephemeral_root: &Path,
        deadline: Instant,
    ) -> Result<Self> {
        Self::create_inner(
            parent,
            checkpoint_id,
            source_roots,
            ephemeral_root,
            deadline,
            None,
        )
    }
    /// 共享父目录须事先配置为当前生产者 UID、专用读取组 GID、0750；本函数不改部署权限。
    /// 原始导出与中间目录仍为 0700，只有过滤封存复验后才开放最终根。
    pub fn create_for_reader(
        parent: &Path,
        checkpoint_id: CheckpointId,
        source_roots: &[PathBuf],
        ephemeral_root: &Path,
        reader_gid: u32,
        deadline: Instant,
    ) -> Result<Self> {
        Self::create_inner(
            parent,
            checkpoint_id,
            source_roots,
            ephemeral_root,
            deadline,
            Some(ReaderIdentity::producer(reader_gid)?),
        )
    }
    fn create_inner(
        parent: &Path,
        checkpoint_id: CheckpointId,
        source_roots: &[PathBuf],
        ephemeral_root: &Path,
        deadline: Instant,
        reader: Option<ReaderIdentity>,
    ) -> Result<Self> {
        check_deadline(deadline)?;
        if source_roots.is_empty() {
            return Err(StagingError::UnsafeRoot);
        }
        let parent_dir = match reader {
            Some(identity) => Directory::publisher(parent, identity)?,
            None => Directory::open(parent, true)?,
        };
        let ephemeral = Directory::open(ephemeral_root, true)?;
        io::disjoint(parent, &parent_dir, ephemeral_root, &ephemeral)?;
        let mut protected = Vec::new();
        for path in source_roots {
            let root = Directory::open(path, false)?;
            io::disjoint(parent, &parent_dir, path, &root)?;
            io::disjoint(ephemeral_root, &ephemeral, path, &root)?;
            protected.push((path.clone(), root));
        }
        let key = digest(checkpoint_id.as_str().as_bytes());
        let export_name = format!("export-{key}");
        let pending_name = format!("pending-{key}");
        let final_name = format!("sealed-{key}");
        if parent_dir.exists(&final_name)? {
            return Err(StagingError::AlreadyExists);
        }
        let (lifecycle, exports, pending, data) =
            creation::layout(&parent_dir, &key, &export_name, &pending_name)?;
        let mut area = Self {
            lifecycle,
            parent: parent_dir,
            exports,
            pending,
            data,
            parent_path: parent.to_owned(),
            export_path: parent.join(&export_name),
            export_name,
            pending_name,
            final_name,
            checkpoint_id,
            protected,
            ephemeral: (ephemeral_root.to_owned(), ephemeral),
            deadline,
            reader,
        };
        if let Err(error) = area.check() {
            area.cleanup()?;
            return Err(error);
        }
        Ok(area)
    }
    /// 仅供原 owner 导出独立副本；此路径永远不是 worker 的传输源。
    #[must_use]
    pub fn export_root(&self) -> &Path {
        &self.export_path
    }
    /// 总冻结窗口不能因后续复制而重新计时。
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    fn check(&self) -> Result<()> {
        check_deadline(self.deadline)?;
        self.parent.check_path(&self.parent_path)?;
        self.lifecycle.check(&self.parent)?;
        self.exports.check_path(&self.export_path)?;
        self.pending
            .check_path(&self.parent_path.join(&self.pending_name))?;
        self.data.check_child(&self.pending, "data")?;
        self.ephemeral.1.check_path(&self.ephemeral.0)?;
        for (path, directory) in &self.protected {
            directory.check_path(path)?;
        }
        Ok(())
    }
}
impl Drop for StagingArea {
    fn drop(&mut self) {
        // 原 FD/根身份与固定预算限定了清理范围；失败不许改删被替换的目录。
        // 普通错误经 seal/abort 显式返回；展开期间仍尽力回收，失败标记持久保留。
        let _ = self.cleanup();
    }
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(StagingError::Expired)
    } else {
        Ok(())
    }
}
type Result<T> = std::result::Result<T, StagingError>;
/// 只返回固定类别；错误不带正文、密钥或宿主机路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StagingError {
    /// 同一封存父目录已有正在构造或回收的生产者。
    Busy,
    /// 前次中断或清理失败；须先完成受控恢复，不能开始新作业。
    CleanupRequired,
    /// 无法确认本次原目录已回收；未完成标记保持关闭。
    CleanupFailed,
    /// 路径、权限、链接或物理重叠不满足隔离要求。
    UnsafeRoot,
    /// checkpoint 名称已被一次尝试占用。
    AlreadyExists,
    /// 来源清单漏项、多项或元数据不完整。
    Catalog,
    /// 实际字节或身份变化，不能完成封存。
    Changed,
    /// 目录、文件、空间或元数据预算已用尽。
    Limit,
    /// 同一个全局冻结截止时间已到。
    Expired,
    /// 文件系统无法完成必需操作。
    Unavailable,
    /// 清单或票据不一致。
    Manifest,
}
impl std::fmt::Display for StagingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "备份封存未完成：{self:?}")
    }
}
impl std::error::Error for StagingError {}
