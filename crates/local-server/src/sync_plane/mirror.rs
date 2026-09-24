//! 镜像视图（idmapped bind，ADR-025 D7／D8）的观察、正文读出与安装。
//!
//! 只在静止窗口内读写镜像；下载是原地写入，运行期间最终路径可见半截内容（P2 结论 2）。
//! 逐段 NOFOLLOW 打开、节点分类与目录枚举借用 `stage.rs` 的助手；`observe.rs`／`content.rs`
//! 只编排（流式 sha256、清单、分帧），不持有任何打开路径的能力。本文件自己只保留两处
//! 固定形态：安装 = stage 对象 `renameat` 进视图并 `fsync` 父目录；删除 = 保全进 stage 后 unlink。

pub mod stage;

mod content;
mod observe;

pub use content::{PlaneContent, StageSink};
pub use observe::{MirrorObservation, observe};

use super::PlaneError;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{fsync, renameat};
use std::path::{Path, PathBuf};
use studiolo_core::VaultPath;
use studiolo_sync_bridge::FileVersion;
use studiolo_sync_bridge::executor::ObjectId;

/// 已核验的镜像视图根：路径加取证时刻的 `(dev, ino)`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MirrorView {
    root: PathBuf,
    identity: (u64, u64),
}

impl MirrorView {
    /// 由静止票据给出的视图根与身份构造；每次使用前复验身份未变。
    #[must_use]
    pub fn new(root: PathBuf, identity: (u64, u64)) -> Self {
        Self { root, identity }
    }

    /// 视图根。
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 取证时刻的 `(dev, ino)`。
    #[must_use]
    pub fn identity(&self) -> (u64, u64) {
        self.identity
    }
}

/// 打开视图根并复验取证时刻的 `(dev, ino)`；漂移即拒绝。
fn open_view(view: &MirrorView) -> Result<OwnedFd, PlaneError> {
    let (root, stat) =
        stage::open_directory_path(view.root()).map_err(|_| PlaneError::Observation)?;
    if (stat.st_dev, stat.st_ino) != view.identity() {
        return Err(PlaneError::Observation);
    }
    Ok(root)
}

/// 把逻辑路径拆成父目录分量与最后一个名字。
fn split(path: &VaultPath) -> (Vec<&str>, &str) {
    let mut parts: Vec<&str> = path.as_str().split('/').collect();
    let name = parts.pop().unwrap_or_default();
    (parts, name)
}

/// 从 `root` 逐级下探到父目录；`create` 时缺失的层级以 0700 建出。
/// 返回 `None` 表示父链缺失（不建时）；链接或非目录一律 `Observation`。
fn descend(root: OwnedFd, parents: &[&str], create: bool) -> Result<Option<OwnedFd>, PlaneError> {
    let mut current = root;
    for name in parents {
        let opened =
            stage::open_node_at(current.as_fd(), *name).map_err(|_| PlaneError::Observation)?;
        current = match opened {
            stage::Opened::Directory(next) => next,
            stage::Opened::Missing if create => created(current.as_fd(), name)?,
            stage::Opened::Missing => return Ok(None),
            stage::Opened::Regular { .. } | stage::Opened::Other => {
                return Err(PlaneError::Observation);
            }
        };
    }
    Ok(Some(current))
}

fn created(dir: BorrowedFd<'_>, name: &str) -> Result<OwnedFd, PlaneError> {
    stage::make_directory_at(dir, name, false).map_err(|_| PlaneError::Storage)?;
    match stage::open_node_at(dir, name).map_err(|_| PlaneError::Storage)? {
        stage::Opened::Directory(next) => Ok(next),
        _ => Err(PlaneError::Storage),
    }
}

/// 父目录里的一个普通单链接文件：缺失 `None`；链接、目录或多链接一律 `Observation`。
fn regular_at(dir: BorrowedFd<'_>, name: &str) -> Result<Option<(OwnedFd, u64)>, PlaneError> {
    match stage::open_node_at(dir, name).map_err(|_| PlaneError::Observation)? {
        stage::Opened::Missing => Ok(None),
        stage::Opened::Regular {
            file,
            bytes,
            unique: true,
        } => Ok(Some((file, bytes))),
        _ => Err(PlaneError::Observation),
    }
}

/// 已打开文件是否与冻结版本逐字节一致。
fn same_version(
    file: BorrowedFd<'_>,
    bytes: u64,
    expected: &FileVersion,
) -> Result<bool, PlaneError> {
    if bytes != expected.bytes {
        return Ok(false);
    }
    let (digest, read) = content::stream(file, expected.bytes, None)?;
    Ok(read == expected.bytes && digest == expected.sha256)
}

/// 把已封口的 stage 对象原子安装进视图：先复验视图身份与目标当前版本
/// （`target_expected`，缺失即须不存在），再 `renameat` 进目标路径并复验。
///
/// # Errors
/// 目标版本不符返回 `Observation`（执行器记 NoEffect）；stage 缺对象或父目录建不出返回
/// `Storage`；安装后复验失败返回 `Unknown`。
pub fn install(
    view: &MirrorView,
    stage: &mut stage::StageStore,
    object: ObjectId,
    target: &VaultPath,
    target_expected: Option<&FileVersion>,
) -> Result<FileVersion, PlaneError> {
    let (parents, name) = split(target);
    let parent = descend(open_view(view)?, &parents, true)?.ok_or(PlaneError::Storage)?;
    match (regular_at(parent.as_fd(), name)?, target_expected) {
        (None, None) => {}
        (Some((file, bytes)), Some(expected)) => {
            if !same_version(file.as_fd(), bytes, expected)? {
                return Err(PlaneError::Observation);
            }
        }
        _ => return Err(PlaneError::Observation),
    }
    let source = stage::hex(object.digest());
    renameat(stage.objects(), source.as_str(), &parent, name).map_err(|_| PlaneError::Storage)?;
    fsync(&parent).map_err(|_| PlaneError::Unknown)?;
    let (installed, bytes) = regular_at(parent.as_fd(), name)
        .map_err(|_| PlaneError::Unknown)?
        .ok_or(PlaneError::Unknown)?;
    let (digest, read) =
        content::stream(installed.as_fd(), bytes, None).map_err(|_| PlaneError::Unknown)?;
    if read != bytes || digest != object.digest() {
        return Err(PlaneError::Unknown);
    }
    Ok(FileVersion {
        revision: digest,
        sha256: digest,
        bytes,
    })
}

/// 删除视图内一个路径：先把当前字节按地址保全进 stage（删除前保全，7 天保留），再 unlink。
///
/// # Errors
/// 目标缺失或版本不符返回 `Observation`；保全失败返回 `Storage`；unlink 时目标已消失返回
/// `Unknown`，其它 unlink 失败返回 `Unavailable`。
pub fn remove(
    view: &MirrorView,
    stage: &mut stage::StageStore,
    target: &VaultPath,
    target_expected: &FileVersion,
    plan_sequence: u64,
    now_unix: i64,
) -> Result<ObjectId, PlaneError> {
    let (parents, name) = split(target);
    let parent = descend(open_view(view)?, &parents, false)?.ok_or(PlaneError::Observation)?;
    let (file, bytes) = regular_at(parent.as_fd(), name)?.ok_or(PlaneError::Observation)?;
    if bytes != target_expected.bytes {
        return Err(PlaneError::Observation);
    }
    let id = ObjectId::from_digest(target_expected.sha256);
    stage.begin()?;
    match content::stream(file.as_fd(), bytes, Some(&mut *stage)) {
        Ok((digest, read)) if read == bytes && digest == target_expected.sha256 => {
            stage.finish(id)?;
        }
        Ok(_) => {
            stage.discard()?;
            return Err(PlaneError::Observation);
        }
        Err(error) => {
            stage.discard()?;
            return Err(error);
        }
    }
    stage.preserve(id, plan_sequence, target.as_str(), now_unix)?;
    drop(file);
    match stage::remove_at(parent.as_fd(), name) {
        Ok(true) => Ok(id),
        Ok(false) => Err(PlaneError::Unknown),
        Err(_) => Err(PlaneError::Unavailable),
    }
}
