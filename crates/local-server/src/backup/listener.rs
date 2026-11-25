//! 备份 socket 的文件系统一侧：父目录属主是本进程、属组是专用读取组、权限 0750；
//! socket 建好后交给读取组、权限 0660。不建目录；同名节点只接管上一任留下、判得死的 socket
//! （`unix_ingress::leftover`），别的一律拒绝；退出时只删仍是本次 inode 的 socket。
//!
//! worker 连接前会按同一组条件核对父目录和 socket（`studiolo-backup-worker` 的 endpoint proof），
//! 两边条件不一致时 worker 那边拒绝连接，所以这里的数字改动要和它一起改。
use crate::{CanonicalWorkbench, unix_ingress::leftover::Entrance};
use rustix::{
    fs::{AtFlags, FileType, Mode, OFlags, Stat, chmodat, chownat, fstat, open, statat, unlinkat},
    io::Errno,
    process::{Gid, geteuid},
};
use std::{
    ffi::OsString,
    os::{
        fd::{AsFd as _, OwnedFd},
        unix::net::UnixListener,
    },
    path::Path,
};

/// 备份 socket 起不来的固定类别；不带路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListenError {
    /// 路径、属主、属组或权限不符合约定，或者同名节点已经存在、又不是判得死的残留。
    Configuration,
    /// 监听没有建立起来。
    Unavailable,
}
impl std::fmt::Display for ListenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "备份 socket 未就绪：{self:?}")
    }
}
impl std::error::Error for ListenError {}

/// 父目录的原句柄加 socket 的名字；之后的核对、改权限和清理都相对这个句柄做。
struct Node {
    parent: OwnedFd,
    name: OsString,
}
impl Node {
    fn stat(&self) -> Result<Stat, Errno> {
        statat(&self.parent, &self.name, AtFlags::SYMLINK_NOFOLLOW)
    }

    /// 本进程建的、只有一个名字的 socket。
    fn socket(&self, uid: u32) -> Result<Stat, ListenError> {
        let stat = self.stat().map_err(|_| ListenError::Configuration)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Socket
            || stat.st_uid != uid
            || stat.st_nlink != 1
        {
            return Err(ListenError::Configuration);
        }
        Ok(stat)
    }
}

/// 本次 socket 的清理责任；销毁时只删仍是同一个 inode 的那个节点。
pub(super) struct Claim {
    node: Node,
    uid: u32,
    identity: (u64, u64),
}
impl Drop for Claim {
    fn drop(&mut self) {
        if self
            .node
            .socket(self.uid)
            .is_ok_and(|stat| identity(&stat) == self.identity)
        {
            let _ = unlinkat(&self.node.parent, &self.node.name, AtFlags::empty());
        }
    }
}

/// 在固定路径上建立监听。任何一步不符就失败，已经建出来的 socket 由 `Claim` 的销毁收走。
pub(super) fn bind(
    path: &Path,
    reader_gid: u32,
    workbench: &CanonicalWorkbench,
) -> Result<(UnixListener, Claim), ListenError> {
    let (Some(directory), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(ListenError::Configuration);
    };
    let parent = open(
        directory,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| ListenError::Configuration)?;
    let uid = geteuid().as_raw();
    let held = fstat(&parent).map_err(|_| ListenError::Configuration)?;
    if FileType::from_raw_mode(held.st_mode) != FileType::Directory
        || held.st_uid != uid
        || held.st_gid != reader_gid
        || held.st_mode & 0o7777 != 0o750
    {
        return Err(ListenError::Configuration);
    }
    let node = Node {
        parent,
        name: name.to_owned(),
    };
    // 上一任被强杀后留下、已经没人监听的 socket 先接管；判不准就停在这里，节点原样留着。
    Entrance {
        parent: node.parent.as_fd(),
        name: &node.name,
        path,
        label: "备份入口",
    }
    .take_over(workbench)
    .map_err(|_| ListenError::Configuration)?;
    // 接管之后还有同名节点，一律拒绝。
    if !matches!(node.stat(), Err(Errno::NOENT)) {
        return Err(ListenError::Configuration);
    }
    let listener = UnixListener::bind(path).map_err(|_| ListenError::Unavailable)?;
    // 路径中途被换掉时，新 socket 不在原父目录里：这里核不过，也不去动别处的节点。
    let created = node.socket(uid)?;
    let claim = Claim {
        identity: identity(&created),
        node,
        uid,
    };
    // 父目录只有本进程能增删节点（属主是自己、0750），改属组和权限期间节点不会被别的账号换掉。
    chownat(
        &claim.node.parent,
        &claim.node.name,
        None,
        Some(Gid::from_raw(reader_gid)),
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(|_| ListenError::Configuration)?;
    chmodat(
        &claim.node.parent,
        &claim.node.name,
        Mode::from_raw_mode(0o660),
        AtFlags::empty(),
    )
    .map_err(|_| ListenError::Configuration)?;
    let ready = claim.node.socket(uid)?;
    if identity(&ready) != claim.identity
        || ready.st_gid != reader_gid
        || ready.st_mode & 0o7777 != 0o660
    {
        return Err(ListenError::Configuration);
    }
    Ok((listener, claim))
}

fn identity(stat: &Stat) -> (u64, u64) {
    (stat.st_dev, stat.st_ino)
}
