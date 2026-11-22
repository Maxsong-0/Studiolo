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
use studiolo_core::checkpoint::{CheckpointOwner, SealedOwnerCheckpoint, WalCheckpointProof};

/// 每次冻结使用独立连接持有 SQLite RESERVED 锁，进程退出即释放；另一条连接开着读事务，把库定在冻结那一刻。
/// 调用方须先排空上游 owner 和人侧完整操作（含审计），再进入本屏障。
/// [`Self::release`] 放开写锁之后，导出读的仍是定格那一刻（BUG-045：拷贝和校验挪到放开之后）。
/// 导出失败的目标留作不可用残片，绝不自动覆盖或删除。
pub struct PolicyCheckpointHandle {
    directory: PathBuf,
    source: Database,
    barrier: Mutex<Option<Frozen>>,
}

/// 一次冻结：写锁连接（放开之后是 None）、定格的读连接、冻结时取的 WAL 证明。
struct Frozen {
    reserved: Option<Database>,
    pinned: Database,
    wal: WalCheckpointProof,
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

    /// 在截止时间前取得跨进程写屏障，把 WAL 整个回写主库，再在另一条连接上定格；
    /// 重复 freeze 拒绝，不收养别的窗口。
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
        // 写锁在手、没有别的写者：WAL 回写完再定格，定格之后旧读者挡不住的就只剩冻结之后的新帧。
        let wal = loop {
            check_deadline(deadline)?;
            match self.source.wal_checkpoint() {
                Ok(proof) if proof.is_complete() => break proof,
                Ok(_) | Err(PolicyError::Busy) => pause(deadline)?,
                Err(error) => return Err(error),
            }
        };
        let pinned =
            Database::open_until(&self.directory, CreatePolicy::ExistingOnly, Some(deadline))?;
        pinned.pin(deadline)?;
        self.source.storage().check()?;
        check_deadline(deadline)?;
        *barrier = Some(Frozen {
            reserved: Some(connection),
            pinned,
            wal,
        });
        Ok(())
    }

    /// 放开写锁，别的进程从此可以写策略；定格的读连接留着，之后的导出读到的仍是冻结那一刻。
    /// 只能放开一次；没冻结就拒绝。
    pub fn release(&self, deadline: Instant) -> Result<(), PolicyError> {
        let mut barrier = self.lock_until(deadline)?;
        let frozen = barrier.as_mut().ok_or(PolicyError::StoreUnavailable)?;
        let reserved = frozen
            .reserved
            .take()
            .ok_or(PolicyError::StoreUnavailable)?;
        reserved.release()
    }

    /// 目标必须是新的空0700目录；成功票据包含WAL、全内容、草案顺序及持久化证明。
    /// 读的是定格那一刻，放开写锁前后都可以调。
    pub fn export(
        &self,
        directory: &Path,
        deadline: Instant,
    ) -> Result<SealedOwnerCheckpoint, PolicyError> {
        // 整个导出期间持有状态锁，thaw不能提前关掉定格的连接。
        let barrier = self.lock_until(deadline)?;
        let frozen = barrier.as_ref().ok_or(PolicyError::StoreUnavailable)?;
        self.check(frozen)?;
        let expected = snapshot::read(&frozen.pinned, deadline)?;
        let target = Storage::checkpoint_target(directory)?;
        frozen.pinned.backup_into(&target, deadline)?;
        target.sync_created()?;
        check_deadline(deadline)?;
        let copy = Database::open_until(directory, CreatePolicy::ExistingOnly, Some(deadline))?;
        if snapshot::read(&copy, deadline)? != expected {
            return Err(PolicyError::CorruptStore);
        }
        drop(copy);
        target.sync_created()?;
        self.check(frozen)?;
        check_deadline(deadline)?;
        Ok(SealedOwnerCheckpoint {
            owner: CheckpointOwner::Policy,
            schema_version: 1,
            wal: frozen.wal,
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
            .and_then(|frozen| frozen.reserved)
            .map_or(Ok(()), |connection| connection.release());
        if poisoned {
            return Err(PolicyError::StoreUnavailable);
        }
        released
    }

    /// 源身份没换、写锁（放开之前）和定格的读事务都还在。
    fn check(&self, frozen: &Frozen) -> Result<(), PolicyError> {
        if let Some(reserved) = &frozen.reserved {
            reserved.storage().check()?;
        }
        self.source.storage().check()?;
        if !frozen.pinned.pinned()? {
            return Err(PolicyError::StoreUnavailable);
        }
        Ok(())
    }

    fn lock_until(&self, deadline: Instant) -> Result<MutexGuard<'_, Option<Frozen>>, PolicyError> {
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

/// 按封存时同一套规则在一个已打开的库上重算 Policy 水位（schema、现行 effective 的版本和哈希、全局吊销），
/// 只读。新根恢复用它核「放出来的库就是清单里那一份」（T-212）。
pub(crate) fn watermark(
    database: &Database,
    deadline: Instant,
) -> Result<studiolo_core::checkpoint::OwnerWatermark, PolicyError> {
    Ok(snapshot::read(database, deadline)?.watermark)
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
