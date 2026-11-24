//! 固定私有准入文件；锁只活在 FD 上，崩溃后不留下需人工清除的状态。
//!
//! 目录句柄和准入文件身份在第一次取用（写者开库通过校验之后）钉住，此后只经这个句柄
//! 按固定名字取用，不再走路径。原因：共享许可是每个完整操作的第一步，它若要求审计库此刻
//! 健康，审计中断（库文件或目录权限被改、目录被挪走）就会把「照常读、落待补记」的既有
//! 行为一并挡掉。冻结方（独占）仍必须面对健康的审计库，前后各核一次。
use super::Storage;
use crate::{
    AuditError,
    operation::{Lease, OPERATION_RETRY},
};
use std::time::Instant;

#[cfg(unix)]
const GATE: &str = "checkpoint-operations.lock";

/// 开库时钉住的准入位置：原目录句柄加准入文件的设备号与 inode。
#[cfg(unix)]
pub(in crate::sqlite) struct Gate {
    root: std::fs::File,
    identity: (u64, u64),
}

#[cfg(unix)]
fn open_gate(root: &std::fs::File, create: bool) -> rustix::io::Result<std::fs::File> {
    use rustix::fs::{Mode, OFlags, openat};
    let flags = OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    if create {
        openat(
            root,
            GATE,
            flags | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        )
    } else {
        openat(root, GATE, flags, Mode::empty())
    }
    .map(std::fs::File::from)
}

#[cfg(unix)]
impl Storage {
    /// 在截止时间前反复试取；别的进程一直占着不相容的锁就超时失败。
    pub(in crate::sqlite) fn operation_lease(
        &self,
        exclusive: bool,
        deadline: Instant,
    ) -> Result<Lease, AuditError> {
        use std::thread;
        loop {
            if Instant::now() >= deadline {
                return Err(AuditError::Unavailable);
            }
            if let Some(lease) = self.try_operation_lease(exclusive)? {
                if Instant::now() >= deadline {
                    return Err(AuditError::Unavailable);
                }
                return Ok(lease);
            }
            thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(OPERATION_RETRY),
            );
        }
    }

    /// 试取一次、不等待：别的进程正占着不相容的锁时返回 `None`，由调用方决定怎么等。
    pub(in crate::sqlite) fn try_operation_lease(
        &self,
        exclusive: bool,
    ) -> Result<Option<Lease>, AuditError> {
        use rustix::fs::{FlockOperation, flock};
        let gate = match self.operation_gate.get() {
            Some(gate) => gate,
            None => {
                let pinned = self.pin_gate()?;
                self.operation_gate.get_or_init(|| pinned)
            }
        };
        let file = open_gate(&gate.root, false).map_err(|_| AuditError::UnsafeStorage)?;
        // 手里的句柄和此刻按名字取到的文件都必须是钉住的那一个：改名、换文件、改权限都不收养。
        let inspect = |held: &std::fs::File| {
            if exclusive {
                self.check()?;
            }
            let named = open_gate(&gate.root, false).map_err(|_| AuditError::UnsafeStorage)?;
            for found in [held, &named] {
                let found = found.metadata().map_err(|_| AuditError::UnsafeStorage)?;
                self.gate_file(&found, gate.identity)?;
            }
            Ok(())
        };
        inspect(&file)?;
        let operation = if exclusive {
            FlockOperation::NonBlockingLockExclusive
        } else {
            FlockOperation::NonBlockingLockShared
        };
        match flock(&file, operation) {
            Ok(()) => {
                let lease = Lease(file);
                inspect(&lease.0)?;
                Ok(Some(lease))
            }
            Err(rustix::io::Errno::WOULDBLOCK | rustix::io::Errno::INTR) => Ok(None),
            Err(_) => Err(AuditError::Unavailable),
        }
    }

    /// 只在审计库此刻通过完整检查时钉住；缺准入文件就在原目录句柄下新建并落盘。
    fn pin_gate(&self) -> Result<Gate, AuditError> {
        use std::{fs::File, os::unix::fs::MetadataExt};
        self.check()?;
        let root = File::open(&self.root).map_err(|_| AuditError::UnsafeStorage)?;
        let opened = root.metadata().map_err(|_| AuditError::UnsafeStorage)?;
        if !opened.is_dir() || (opened.dev(), opened.ino()) != self.root_identity {
            return Err(AuditError::UnsafeStorage);
        }
        let file = match open_gate(&root, true) {
            Ok(file) => {
                file.sync_all().map_err(|_| AuditError::Unavailable)?;
                root.sync_all().map_err(|_| AuditError::Unavailable)?;
                file
            }
            Err(rustix::io::Errno::EXIST) => {
                open_gate(&root, false).map_err(|_| AuditError::UnsafeStorage)?
            }
            Err(_) => return Err(AuditError::UnsafeStorage),
        };
        let found = file.metadata().map_err(|_| AuditError::UnsafeStorage)?;
        let identity = (found.dev(), found.ino());
        self.gate_file(&found, identity)?;
        Ok(Gate { root, identity })
    }

    fn gate_file(&self, found: &std::fs::Metadata, pinned: (u64, u64)) -> Result<(), AuditError> {
        use std::os::unix::fs::MetadataExt;
        if !found.is_file()
            || found.nlink() != 1
            || found.mode() & 0o7777 != 0o600
            || found.uid() != self.owner
            || found.len() != 0
            || (found.dev(), found.ino()) != pinned
        {
            return Err(AuditError::UnsafeStorage);
        }
        Ok(())
    }
}

#[cfg(not(unix))]
impl Storage {
    pub(in crate::sqlite) fn operation_lease(
        &self,
        _: bool,
        _: Instant,
    ) -> Result<Lease, AuditError> {
        Err(AuditError::UnsupportedPlatform)
    }

    pub(in crate::sqlite) fn try_operation_lease(
        &self,
        _: bool,
    ) -> Result<Option<Lease>, AuditError> {
        Err(AuditError::UnsupportedPlatform)
    }
}
