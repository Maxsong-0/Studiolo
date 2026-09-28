//! 排除项守卫（D10）：审批已去掉的排除文件夹在镜像视图里是否仍有残留。只读、只编排：
//! 根目录经 `recovery::directory_path` 逐段 NOFOLLOW 打开，逐段下探与目录枚举都借 `mirror::stage`
//! 的描述符助手；本文件没有 IO 记号。

use super::super::PlaneError;
use super::super::mirror::stage;
use super::directory_path;
use rustix::fd::{AsFd, OwnedFd};
use std::path::Path;

/// 排除项守卫（D10）：`previous` 里有、`current` 里没有的文件夹，若镜像视图里仍有残留，
/// 返回第一个残留文件夹名；调用方据此拒绝 RunOnce。文件夹不存在即跳过，存在且非空
/// （或名字被文件／链接占着）即残留；名字畸形按残留处理。
///
/// # Errors
/// 视图不可读返回 `Unavailable`。
pub fn residue_guard(
    previous: &[String],
    current: &[String],
    view_root: &Path,
) -> Result<Option<String>, PlaneError> {
    let (root, _) = directory_path(view_root).map_err(|_| PlaneError::Unavailable)?;
    for folder in previous.iter().filter(|folder| !current.contains(folder)) {
        let malformed = studiolo_core::VaultPath::parse(folder).is_err();
        if malformed || residue(&root, folder)? {
            return Ok(Some(folder.clone()));
        }
    }
    Ok(None)
}

fn residue(root: &OwnedFd, folder: &str) -> Result<bool, PlaneError> {
    let mut current: Option<OwnedFd> = None;
    for name in folder.split('/') {
        let dir = current.as_ref().unwrap_or(root).as_fd();
        current = match stage::open_node_at(dir, name) {
            Ok(stage::Opened::Directory(next)) => Some(next),
            Ok(stage::Opened::Missing) => return Ok(false),
            Ok(stage::Opened::Regular { .. } | stage::Opened::Other) => return Ok(true),
            Err(_) => return Err(PlaneError::Unavailable),
        };
    }
    let Some(directory) = current else {
        return Ok(false);
    };
    let names = stage::entry_names(directory.as_fd()).map_err(|_| PlaneError::Unavailable)?;
    Ok(!names.is_empty())
}
