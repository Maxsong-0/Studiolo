//! 策略库跨进程写屏障与已核验的独立副本；不承担其他 owner 的语义排空。

mod snapshot;

use crate::{
    PolicyError,
    database::Database,
    storage::{CreatePolicy, Storage},
};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, TryLockError},
    thread,
    time::{Duration, Instant},
};
use studiolo_core::checkpoint::{CheckpointOwner, SealedOwnerCheckpoint};

/// 每次冻结使用独立连接持有 SQLite RESERVED 锁，进程退出即释放。
/// 调用方须先排空上游 owner 和人侧完整操作（含审计），再进入本屏障。
/// 导出失败的目标留作不可用残片，绝不自动覆盖或删除。
pub struct PolicyCheckpointHandle {
    directory: PathBuf,
    source: Database,
    barrier: Mutex<Option<Database>>,
}

impl PolicyCheckpointHandle {
    /// 只打开已有库；初始化新库仍由主人发布者负责。
    pub fn open_existing(directory: &Path) -> Result<Self, PolicyError> {
        Ok(Self {
            directory: directory.to_owned(),
            source: Database::open(directory, CreatePolicy::ExistingOnly)?,
            barrier: Mutex::new(None),
        })
    }

    /// 在截止时间前取得跨进程写屏障；重复 freeze 拒绝，不收养别的窗口。
    pub fn freeze(&self, deadline: Instant) -> Result<(), PolicyError> {
        let mut barrier = self.lock_until(deadline)?;
        if barrier.is_some() {
            return Err(PolicyError::Busy);
        }
        self.source.storage().check()?;
        let connection =
            Database::open_until(&self.directory, CreatePolicy::ExistingOnly, Some(deadline))?;
        loop {
            check_deadline(deadline)?;
            match connection.reserve(deadline) {
                Ok(()) => break,
                Err(PolicyError::Busy) => pause(deadline)?,
                Err(error) => return Err(error),
            }
        }
        self.source.storage().check()?;
        check_deadline(deadline)?;
        *barrier = Some(connection);
        Ok(())
    }

    /// 目标必须是新的空0700目录；成功票据包含WAL、全内容、草案顺序及持久化证明。
    pub fn export(
        &self,
        directory: &Path,
        deadline: Instant,
    ) -> Result<SealedOwnerCheckpoint, PolicyError> {
        // 整个导出期间持有状态锁，thaw不能提前释放写屏障。
        let barrier = self.lock_until(deadline)?;
        let reserved = barrier.as_ref().ok_or(PolicyError::StoreUnavailable)?;
        reserved.storage().check()?;
        self.source.storage().check()?;
        let wal = loop {
            check_deadline(deadline)?;
            match self.source.wal_checkpoint() {
                Ok(proof) if proof.is_complete() => break proof,
                Ok(_) | Err(PolicyError::Busy) => pause(deadline)?,
                Err(error) => return Err(error),
            }
        };
        let expected = snapshot::read(&self.source, deadline)?;
        let target = Storage::checkpoint_target(directory)?;
        self.source.vacuum_into(&target, deadline)?;
        target.sync_created()?;
        check_deadline(deadline)?;
        let copy = Database::open_until(directory, CreatePolicy::ExistingOnly, Some(deadline))?;
        if snapshot::read(&copy, deadline)? != expected {
            return Err(PolicyError::CorruptStore);
        }
        drop(copy);
        target.sync_created()?;
        reserved.storage().check()?;
        self.source.storage().check()?;
        check_deadline(deadline)?;
        Ok(SealedOwnerCheckpoint {
            owner: CheckpointOwner::Policy,
            schema_version: 1,
            wal,
            target_synced: true,
            parent_directory_synced: true,
            reopen_verified: true,
            source_binding_stable: true,
            watermark: expected.watermark,
        })
    }

    /// 解冻幂等；即使ROLLBACK失败也关闭专用连接，不遗留锁。
    pub fn thaw(&self) -> Result<(), PolicyError> {
        let (mut barrier, poisoned) = match self.barrier.lock() {
            Ok(guard) => (guard, false),
            Err(error) => (error.into_inner(), true),
        };
        let released = barrier
            .take()
            .map_or(Ok(()), |connection| connection.release());
        if poisoned {
            return Err(PolicyError::StoreUnavailable);
        }
        released
    }

    fn lock_until(
        &self,
        deadline: Instant,
    ) -> Result<MutexGuard<'_, Option<Database>>, PolicyError> {
        loop {
            check_deadline(deadline)?;
            match self.barrier.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(TryLockError::Poisoned(_)) => return Err(PolicyError::StoreUnavailable),
                Err(TryLockError::WouldBlock) => pause(deadline)?,
            }
        }
    }
}

fn check_deadline(deadline: Instant) -> Result<(), PolicyError> {
    if Instant::now() >= deadline {
        Err(PolicyError::Busy)
    } else {
        Ok(())
    }
}

fn pause(deadline: Instant) -> Result<(), PolicyError> {
    check_deadline(deadline)?;
    thread::sleep(
        deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(10)),
    );
    Ok(())
}

#[cfg(test)]
#[path = "../tests/checkpoint/private.rs"]
mod tests;
