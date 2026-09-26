//! 外 stage（ADR-025 D8）：`<plane_root>/stage/objects/<sha256 hex>` 按内容寻址落库，
//! 删除前保全的对象与 7 天保留期记在固定名 `stage/index.json`。
//! 对象账本在 `store.rs`（`StageStore`），索引编解码在 `index.rs`；两者都不含系统调用。
//!
//! **本文件是 `sync_plane` 下唯一允许做通用文件 IO 的地方**（架构守卫按固定文件名登记）：
//! 逐段 NOFOLLOW 打开、节点分类、目录枚举、建目录、建文件、删名字、改名与刷盘都只在这里。
//! `mirror.rs` 借用打开路径的助手（`pub(super)`，不出 `mirror` 模块），自己只保留登记过的
//! 安装形态；`recovery.rs` 只借用需要已有描述符的助手，打开路径的能力不外泄。
//! 写法照 `observability` 的 `deadman-state.json`：暂存件 → fsync → renameat → fsync 目录；
//! 读时失败关闭。对象文件 NOFOLLOW、O_EXCL、0600；孤儿在打开与回收时清掉。

mod index;
mod store;

pub use index::{PreservedEntry, StageIndex, hex};
pub(in crate::sync_plane) use index::{read_fully, write_fully};
pub use store::StageStore;

use crate::sync_plane::PlaneError;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{
    AtFlags, CWD, Dir, FileType, Mode, OFlags, Stat, StatxFlags, fstat, fsync, mkdirat, openat,
    renameat, statx, unlinkat,
};
use rustix::io::Errno;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt as _;
use std::path::{Component, Path};

/// 平面私有根下的 stage 目录名。
pub const STAGE_DIR: &str = "stage";
/// stage 内的对象目录名。
pub const OBJECTS_DIR: &str = "objects";
/// 保全索引的固定文件名。
pub const INDEX_FILE: &str = "index.json";
/// 保全索引的唯一暂存件名；只用于完整替换。
pub const INDEX_STAGE_FILE: &str = "index.json.new";
/// 接收中对象的唯一临时名；同一时刻最多一个接收。
pub const RECEIVING_FILE: &str = "receiving.partial";
/// 删除前保全的最短保留期。
pub const RETENTION_SECONDS: u64 = 7 * 24 * 60 * 60;
/// 索引文件字节上限。
pub const INDEX_MAX_BYTES: usize = 1024 * 1024;
/// 单个对象字节上限；与执行器 `CasLimits::internal` 的单对象上限一致。
pub const OBJECT_MAX_BYTES: usize = 256 * 1024 * 1024;
/// 一个目录最多枚举的名字数；超过即失败关闭。
const ENTRY_LIMIT: usize = 65_536;
const DIRECTORY: OFlags = OFlags::RDONLY.union(OFlags::DIRECTORY);

/// 打开一个名字的结果；链接、套接字与其它特殊节点都只报 `Other`，不给描述符。
pub(in crate::sync_plane) enum Opened {
    /// 名字不存在。
    Missing,
    /// 存在但既不是目录也不是普通文件（含符号链接）。
    Other,
    /// 目录，已按 NOFOLLOW 打开。
    Directory(OwnedFd),
    /// 普通文件，已只读打开；`unique` 为链接数是否恰为 1。
    Regular {
        /// 只读描述符。
        file: OwnedFd,
        /// 打开时的字节数。
        bytes: u64,
        /// 链接数是否恰为 1。
        unique: bool,
    },
}

/// 当前进程身份：真实与有效 UID 必须一致，返回有效 UID。
pub(in crate::sync_plane) fn caller() -> Result<u32, PlaneError> {
    use rustix::process::{geteuid, getuid};
    if getuid() != geteuid() {
        return Err(PlaneError::Storage);
    }
    Ok(geteuid().as_raw())
}

/// 唯一的打开点：NOFOLLOW、非阻塞、不夺控制终端、CLOEXEC，并立刻取回 stat。
fn open_with<N: rustix::path::Arg>(
    dir: BorrowedFd<'_>,
    name: N,
    flags: OFlags,
    mode: Mode,
) -> rustix::io::Result<(OwnedFd, Stat)> {
    let common = OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC;
    let handle = openat(dir, name, flags | common, mode)?;
    let stat = fstat(&handle)?;
    Ok((handle, stat))
}

/// 逐段 NOFOLLOW 打开一个绝对目录路径；任一段是链接、不是目录或不是规范分量即失败。
pub(super) fn open_directory_path(path: &Path) -> rustix::io::Result<(OwnedFd, Stat)> {
    if !path.is_absolute() {
        return Err(Errno::INVAL);
    }
    let mut current = open_with(CWD, "/", DIRECTORY, Mode::empty())?;
    for component in path.components() {
        let name = match component {
            Component::RootDir => continue,
            Component::Normal(name) => name,
            _ => return Err(Errno::INVAL),
        };
        current = open_with(current.0.as_fd(), name, DIRECTORY, Mode::empty())?;
    }
    Ok(current)
}

/// 打开目录里的一个名字并分类；`Missing`／`Other` 不带描述符。
pub(in crate::sync_plane) fn open_node_at<N: rustix::path::Arg>(
    dir: BorrowedFd<'_>,
    name: N,
) -> rustix::io::Result<Opened> {
    let (file, stat) = match open_with(dir, name, OFlags::RDONLY, Mode::empty()) {
        Ok(opened) => opened,
        Err(Errno::NOENT) => return Ok(Opened::Missing),
        Err(Errno::LOOP | Errno::NXIO) => return Ok(Opened::Other),
        Err(error) => return Err(error),
    };
    Ok(match FileType::from_raw_mode(stat.st_mode) {
        FileType::Directory => Opened::Directory(file),
        FileType::RegularFile => Opened::Regular {
            file,
            bytes: u64::try_from(stat.st_size).map_err(|_| Errno::INVAL)?,
            unique: stat.st_nlink == 1,
        },
        _ => Opened::Other,
    })
}

/// 目录里的全部名字（不含 `.`／`..`），按字节序排好；超过上限即失败。
pub(in crate::sync_plane) fn entry_names(dir: BorrowedFd<'_>) -> rustix::io::Result<Vec<OsString>> {
    let mut names = Vec::new();
    for entry in Dir::read_from(dir)? {
        let name = entry?.file_name().to_bytes().to_vec();
        if name == b"." || name == b".." {
            continue;
        }
        if names.len() >= ENTRY_LIMIT {
            return Err(Errno::FBIG);
        }
        names.push(OsString::from_vec(name));
    }
    names.sort();
    Ok(names)
}

/// 在目录里建 0700 子目录；`exclusive` 时已存在即错，否则容忍已存在。
pub(super) fn make_directory_at(
    dir: BorrowedFd<'_>,
    name: &str,
    exclusive: bool,
) -> rustix::io::Result<()> {
    match mkdirat(dir, name, Mode::from_raw_mode(0o700)) {
        Err(Errno::EXIST) if !exclusive => Ok(()),
        result => result,
    }
}

/// 独占创建（`exclusive`）或按需创建并读写打开一个 0600 文件；调用方负责核对。
pub(in crate::sync_plane) fn create_at(
    dir: BorrowedFd<'_>,
    name: &str,
    exclusive: bool,
) -> rustix::io::Result<(OwnedFd, Stat)> {
    let flags = if exclusive {
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL
    } else {
        OFlags::RDWR | OFlags::CREATE
    };
    open_with(dir, name, flags, Mode::from_raw_mode(0o600))
}

/// 删除目录里的一个名字；不存在返回 `false`。
pub(in crate::sync_plane) fn remove_at<N: rustix::path::Arg>(
    dir: BorrowedFd<'_>,
    name: N,
) -> rustix::io::Result<bool> {
    match unlinkat(dir, name, AtFlags::empty()) {
        Ok(()) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(error) => Err(error),
    }
}

/// 同一目录内改名（对象封口、索引替换）。
fn rename_within(dir: BorrowedFd<'_>, from: &str, to: &str) -> rustix::io::Result<()> {
    renameat(dir, from, dir, to)
}

/// 把文件内容或目录项刷盘。
fn flush(fd: BorrowedFd<'_>) -> rustix::io::Result<()> {
    fsync(fd)
}

fn private_directory_at(dir: BorrowedFd<'_>, name: &str, euid: u32) -> Result<OwnedFd, PlaneError> {
    let (handle, stat) =
        open_with(dir, name, DIRECTORY, Mode::empty()).map_err(|_| PlaneError::Storage)?;
    if private_directory(&stat, euid) {
        Ok(handle)
    } else {
        Err(PlaneError::Storage)
    }
}

/// 目录是否为本人所有的 0700。
/// 两个已打开目录是否在同一 vfsmount：外 stage 与镜像视图不同挂载时 `renameat` 必然 EXDEV，
/// 安装前先核（ADR-025 D7／D8；置备把 `plane_root` 放在与视图同一条 idmapped bind 之下）。
/// 内核不报 `STATX_MNT_ID` 时按「不同挂载」处理，失败关闭。
pub(in crate::sync_plane) fn same_mount(
    left: BorrowedFd<'_>,
    right: BorrowedFd<'_>,
) -> rustix::io::Result<bool> {
    let first = statx(left, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)?;
    let second = statx(right, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)?;
    let reported = StatxFlags::MNT_ID.bits();
    Ok(first.stx_mask & reported != 0
        && second.stx_mask & reported != 0
        && first.stx_mnt_id == second.stx_mnt_id)
}

pub(in crate::sync_plane) fn private_directory(stat: &Stat, euid: u32) -> bool {
    FileType::from_raw_mode(stat.st_mode) == FileType::Directory
        && stat.st_uid == euid
        && Mode::from_raw_mode(stat.st_mode) == Mode::from_raw_mode(0o700)
}

/// 普通文件是否为本人所有的 0600 单链接且不超限；是则返回字节数。
pub(in crate::sync_plane) fn private_regular(stat: &Stat, euid: u32, max: usize) -> Option<usize> {
    let length = usize::try_from(stat.st_size).ok()?;
    (FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile
        && stat.st_nlink == 1
        && stat.st_uid == euid
        && Mode::from_raw_mode(stat.st_mode) == Mode::from_raw_mode(0o600)
        && length <= max)
        .then_some(length)
}

/// 打开目录里的一个普通文件供整读：不存在返回 `None`；不是本人 0600 单链接或超限即错。
pub(in crate::sync_plane) fn open_regular_at(
    dir: BorrowedFd<'_>,
    name: &str,
    euid: u32,
    max: usize,
) -> rustix::io::Result<Option<(OwnedFd, usize)>> {
    let (file, stat) = match open_with(dir, name, OFlags::RDONLY, Mode::empty()) {
        Ok(opened) => opened,
        Err(Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error),
    };
    let length = private_regular(&stat, euid, max).ok_or(Errno::PERM)?;
    Ok(Some((file, length)))
}

/// 完整替换目录里的固定文件：清掉陈旧暂存件 → 独占建暂存件 → 写满 → fsync →
/// renameat → fsync 目录。中途失败只会留下暂存件，不会留下半个目标文件。
pub(in crate::sync_plane) fn replace_file(
    dir: BorrowedFd<'_>,
    staging: &str,
    target: &str,
    bytes: &[u8],
) -> rustix::io::Result<()> {
    remove_at(dir, staging)?;
    let (file, _) = create_at(dir, staging, true)?;
    write_fully(file.as_fd(), bytes)?;
    flush(file.as_fd())?;
    drop(file);
    rename_within(dir, staging, target)?;
    flush(dir)
}
