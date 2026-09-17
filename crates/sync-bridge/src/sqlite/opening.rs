//! 分表 SQLite owner；验证缓存绑定文件 pins、SQL revision 和 data_version。

use super::{BridgeError, configuration, database, model::*, storage::Storage, validate};
use crate::{Baseline, Binding, Digest, Limits, PlanningCursor};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::{cell::Cell, collections::BTreeMap, path::Path};

/// 先 drop SQLite 再释放真实文件租约；不持有 canonical/mirror 文件能力。
pub struct BridgeState {
    // 字段按声明顺序析构：Connection 必须先于 Storage/HeldLease 关闭。
    pub(super) connection: Connection,
    pub(super) storage: Storage,
    pub(super) epoch: Digest,
    expected: Binding,
    pub(super) cache: State,
    pub(super) data_version: i64,
    pub(super) poisoned: Cell<bool>,
    #[cfg(test)]
    pub(super) stats: Statistics,
    #[cfg(test)]
    pub(super) before_guard: Option<Box<dyn FnOnce()>>,
    #[cfg(test)]
    pub(super) after_commit: Option<Box<dyn FnOnce()>>,
}
#[cfg(test)]
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Statistics {
    pub full_loads: usize,
    pub full_writes: usize,
    pub action_writes: usize,
}

impl BridgeState {
    /// 目录须预先批准且已存在；任何残留均拒绝初始化。
    pub fn initialize(root: &Path, binding: Binding, limits: Limits) -> Result<Self> {
        let state = State {
            version: 0,
            cursor: PlanningCursor {
                binding: binding.clone(),
                baseline_version: 0,
                last_plan_sequence: 0,
                last_canonical_observation: 0,
                last_mirror_observation: 0,
            },
            baseline: Baseline {
                binding: binding.clone(),
                version: 0,
                canonical_observation: 0,
                mirror_observation: 0,
                entries: BTreeMap::new(),
            },
            binding: binding.clone(),
            limits,
            pending: None,
            last: None,
            floor: 0,
            retirement: None,
            fuse: None,
        };
        validate::state(&state)?;
        Self::open(root, binding, Some(state))
    }
    /// 不建主库/lease，不升级 schema；允许既有合法 SQLite 恢复侧文件。
    pub fn open_existing(root: &Path, expected: Binding) -> Result<Self> {
        Self::open(root, expected, None)
    }
    fn open(root: &Path, expected: Binding, initial: Option<State>) -> Result<Self> {
        // 局部按反声明顺序析构：任一 ? 失败先销毁 tx/Connection，再释放 Storage。
        // 成功移动进 BridgeState 后则由上面的字段顺序保证相同的关闭顺序。
        let storage = Storage::open(root, initial.is_some())?;
        storage.check()?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE;
        let mut connection =
            Connection::open_with_flags(&storage.database, flags).map_err(database::error)?;
        storage.check()?;
        // 拒绝打开也不能在 Drop 时主动 checkpoint；必要的 SQLite hot 恢复由引擎负责。
        connection
            .set_db_config(
                rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
                true,
            )
            .map_err(database::error)?;
        connection
            .execute_batch(
                "PRAGMA trusted_schema=OFF;PRAGMA foreign_keys=ON;PRAGMA busy_timeout=5000;",
            )
            .map_err(database::error)?;
        if let Some(state) = initial {
            configuration::connection(&connection)?;
            connection
                .execute_batch("PRAGMA page_size=4096;PRAGMA max_page_count=65536;")
                .map_err(database::error)?;
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database::error)?;
            database::initialize(&tx, &state)?;
            storage.check()?;
            tx.commit().map_err(database::error)?;
            storage.check()?;
            let mode: String = connection
                .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
                .map_err(database::error)?;
            if mode != "wal" {
                return Err(BridgeError::Unavailable);
            }
        }
        let data_version = data_version(&connection)?;
        // existing 路径首先取得一致读快照：坏库/错误 binding 不改模式或主动刷 WAL。
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database::error)?;
        database::schema(&tx)?;
        let cache = database::load(&tx)?;
        if cache.binding != expected {
            return Err(BridgeError::BindingMismatch);
        }
        database::integrity(&tx)?;
        storage.check()?;
        tx.commit().map_err(database::error)?;
        if self::data_version(&connection)? != data_version {
            return Err(BridgeError::VersionConflict);
        }
        configuration::connection(&connection)?;
        connection.execute_batch("PRAGMA max_page_count=65536;PRAGMA journal_size_limit=0;PRAGMA wal_autocheckpoint=1;").map_err(database::error)?;
        configuration::verify(&connection)?;
        storage.check()?;
        checkpoint(&connection)?;
        storage.check()?;
        if self::data_version(&connection)? != data_version
            || database::revision(&connection)? != cache.version
        {
            return Err(BridgeError::VersionConflict);
        }
        let mut epoch = [0; 32];
        getrandom::fill(&mut epoch).map_err(|_| BridgeError::Unavailable)?;
        let result = Self {
            connection,
            storage,
            epoch,
            expected,
            cache,
            data_version,
            poisoned: Cell::new(false),
            #[cfg(test)]
            stats: Statistics {
                full_loads: 1,
                ..Statistics::default()
            },
            #[cfg(test)]
            before_guard: None,
            #[cfg(test)]
            after_commit: None,
        };
        result.read()?;
        Ok(result)
    }
    /// 本次打开实例核验标识不是权限；过期/跨实例观察不能用于新执行。
    pub fn observation_epoch(&self) -> Digest {
        self.epoch
    }
    /// 核验当前文件、SQL 版本和缓存绑定后返回有界控制状态。
    pub fn view(&self) -> Result<StateView> {
        Ok(self.read()?.view())
    }
    #[cfg(test)]
    pub(super) fn statistics(&self) -> Statistics {
        self.stats
    }
    pub(super) fn read(&self) -> Result<&State> {
        if self.poisoned.get() {
            return Err(BridgeError::Unavailable);
        }
        self.storage.check()?;
        if self.cache.binding != self.expected {
            return Err(BridgeError::BindingMismatch);
        }
        if data_version(&self.connection)? != self.data_version
            || database::revision(&self.connection)? != self.cache.version
        {
            self.poisoned.set(true);
            return Err(BridgeError::VersionConflict);
        }
        self.storage.check()?;
        if data_version(&self.connection)? != self.data_version {
            self.poisoned.set(true);
            return Err(BridgeError::VersionConflict);
        }
        Ok(&self.cache)
    }
}
pub(super) fn data_version(db: &Connection) -> Result<i64> {
    db.query_row("PRAGMA data_version", [], |r| r.get(0))
        .map_err(database::error)
}
pub(super) fn checkpoint(db: &Connection) -> Result<()> {
    let (busy, log, left): (i64, i64, i64) = db
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .map_err(database::error)?;
    if busy != 0 || log != 0 || left != 0 {
        return Err(BridgeError::Unavailable);
    }
    Ok(())
}
