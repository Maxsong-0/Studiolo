//! 上一任被强杀后留下的 socket：判死才接管，判不准一律拒绝、一个不删（T-231）。
//!
//! 判死靠三样，缺一样都不动：
//! 1. 本进程握着主库 Vault 根目录的内核锁（[`SoleInstance`]）：上一任死了锁才放得开，活着的另一任拿着它；
//! 2. 节点是本账号的、只有一个名字的 socket，原父目录里看到的和按路径看到的是同一个；
//! 3. 不等待地连一下，**只有** `ECONNREFUSED` 算没人监听。连上、队列满、类型不符、别的错误都算判不准。
//!
//! 删之前再核一遍还是判死时的那一个，删除相对原父目录句柄做。和入口其余部分一样，不宣称对抗同 UID 的进程。

use crate::{CanonicalWorkbench, coordinator::SoleInstance};
use rustix::{
    fs::{AtFlags, CWD, FileType, Stat, statat, unlinkat},
    io::Errno,
    process::geteuid,
};
use std::{ffi::OsStr, os::fd::BorrowedFd, path::Path};

/// 判不准，或者删不掉；不带路径。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Refused;

/// 一个入口的位置：父目录的原句柄、名字、整条路径，和日志里的叫法。
pub(crate) struct Entrance<'a> {
    pub(crate) parent: BorrowedFd<'a>,
    pub(crate) name: &'a OsStr,
    pub(crate) path: &'a Path,
    pub(crate) label: &'static str,
}

impl Entrance<'_> {
    /// 没有同名节点返回 `None`；看不清楚就拒绝。
    fn node(&self) -> Result<Option<Stat>, Refused> {
        match statat(self.parent, self.name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => Ok(Some(stat)),
            Err(Errno::NOENT) => Ok(None),
            Err(_) => Err(Refused),
        }
    }

    /// 原父目录里的节点合格，而且按路径看到的就是它：连接走的是路径，两边不是同一个就不能拿来判死。
    fn candidate(&self) -> Result<Stat, Refused> {
        let stat = self.node()?.ok_or(Refused)?;
        let by_path = statat(CWD, self.path, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| Refused)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Socket
            || stat.st_uid != geteuid().as_raw()
            || stat.st_nlink != 1
            || !same(&stat, &by_path)
        {
            return Err(Refused);
        }
        Ok(stat)
    }

    /// 只看不动：这个节点现在能不能判死。`probe` 回答「没人监听」。
    fn judge(&self, probe: impl FnOnce(&Path) -> bool) -> Result<Stat, Refused> {
        let found = self.candidate()?;
        if !probe(self.path) {
            return Err(Refused);
        }
        // 探活的空当里换了节点，刚才的结论就不是说它的。
        if !same(&found, &self.candidate()?) {
            return Err(Refused);
        }
        Ok(found)
    }

    /// 没有节点，或者是判得死的残留；不删。启动早期用它尽早拒绝，真正接管时还要再判一遍。
    pub(crate) fn acceptable(&self) -> Result<(), Refused> {
        if self.node()?.is_some() {
            self.judge(unheard)?;
        }
        Ok(())
    }

    /// 没有节点就什么都不做；有就要先拿到「唯一一任」的凭据，判死了才删，并留一行日志。
    pub(crate) fn take_over(&self, workbench: &CanonicalWorkbench) -> Result<(), Refused> {
        if self.node()?.is_none() {
            return Ok(());
        }
        let sole = workbench.sole_instance().map_err(|_| Refused)?;
        let retired = self.retire_leftover(&sole, unheard)?;
        eprintln!("{}", self.line(&retired));
        Ok(())
    }

    /// 删掉判死的这一个节点。凭据在删完之前一直借着：期间主库锁放不开。
    pub(crate) fn retire_leftover(
        &self,
        _sole: &SoleInstance<'_>,
        probe: impl FnOnce(&Path) -> bool,
    ) -> Result<Stat, Refused> {
        let found = self.judge(probe)?;
        unlinkat(self.parent, self.name, AtFlags::empty()).map_err(|_| Refused)?;
        Ok(found)
    }

    /// 接管了什么，够事后对上是哪一任留下的；不带路径。建的时刻取 mtime：后来的改权限、改属组不动它。
    fn line(&self, stat: &Stat) -> String {
        let made = jiff::Timestamp::from_second(stat.st_mtime)
            .map_or_else(|_| "未知".to_owned(), |time| time.to_string());
        format!(
            "接管上一任留下的{} socket（inode {}，权限 {:04o}，建于 {made}）",
            self.label,
            stat.st_ino,
            stat.st_mode & 0o7777
        )
    }
}

/// 同一个节点：名字换了对象、对象被改过（权限、属主、硬链接）都会动 ctime，也算不同。
fn same(left: &Stat, right: &Stat) -> bool {
    left.st_dev == right.st_dev
        && left.st_ino == right.st_ino
        && left.st_ctime == right.st_ctime
        && left.st_ctime_nsec == right.st_ctime_nsec
}

/// 没人监听：不等待地连一下，只认 `ECONNREFUSED`。
#[cfg(target_os = "linux")]
fn unheard(path: &Path) -> bool {
    use rustix::net::{
        AddressFamily, SocketAddrUnix, SocketFlags, SocketType, connect, socket_with,
    };
    let Ok(address) = SocketAddrUnix::new(path) else {
        return false;
    };
    let Ok(probe) = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    ) else {
        return false;
    };
    matches!(connect(&probe, &address), Err(Errno::CONNREFUSED))
}

/// 别的平台上队列满也回 `ECONNREFUSED`，分不出死活：一律当作判不准。
#[cfg(not(target_os = "linux"))]
fn unheard(_: &Path) -> bool {
    false
}
