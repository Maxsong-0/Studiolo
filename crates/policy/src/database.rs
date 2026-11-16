//! 策略私有库 `policy.sqlite3` 的唯一打开点；主人句柄与接入方句柄共用同一连接封装。
//!
//! 写事务一律 `BEGIN IMMEDIATE`，读事务用 DEFERRED 快照，使一次调用里读到的多张表彼此一致。

pub(crate) mod schema;

use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::{
    PolicyError,
    storage::{CreatePolicy, Storage},
};

pub(crate) struct Database {
    connection: Mutex<Connection>,
    storage: Storage,
}

impl Database {
    pub(crate) fn open(directory: &Path, policy: CreatePolicy) -> Result<Self, PolicyError> {
        Self::open_until(directory, policy, None)
    }

    pub(crate) fn open_until(
        directory: &Path,
        policy: CreatePolicy,
        deadline: Option<Instant>,
    ) -> Result<Self, PolicyError> {
        let storage = Storage::prepare(directory, policy)?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE;
        let mut connection =
            Connection::open_with_flags(&storage.database, flags).map_err(unavailable)?;
        let timeout = deadline.map_or(Duration::from_secs(5), |until| {
            until
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(250))
        });
        connection.busy_timeout(timeout).map_err(unavailable)?;
        if let Some(until) = deadline {
            if Instant::now() >= until {
                return Err(PolicyError::Busy);
            }
            connection
                .progress_handler(1000, Some(move || Instant::now() >= until))
                .map_err(unavailable)?;
        }
        connection
            .execute_batch(
                "PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;",
            )
            .map_err(unavailable)?;
        {
            let transaction = connection
                .transaction_with_behavior(if storage.created {
                    TransactionBehavior::Immediate
                } else {
                    TransactionBehavior::Deferred
                })
                .map_err(unavailable)?;
            schema::initialize(&transaction, storage.created)?;
            storage.check()?;
            transaction.commit().map_err(unavailable)?;
        }
        let mut mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(unavailable)?;
        if mode != "wal" {
            mode = connection
                .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
                .map_err(unavailable)?;
        }
        if mode != "wal" {
            return Err(PolicyError::StoreUnavailable);
        }
        storage.check()?;
        connection
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(unavailable)?;
        if deadline.is_some_and(|until| Instant::now() >= until) {
            return Err(PolicyError::Busy);
        }
        Ok(Self {
            connection: Mutex::new(connection),
            storage,
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, PolicyError> {
        self.storage.check()?;
        self.connection
            .lock()
            .map_err(|_| PolicyError::StoreUnavailable)
    }

    pub(crate) fn storage(&self) -> &Storage {
        &self.storage
    }

    /// checkpoint 专用连接；拿到锁后不再通过 read/write 开嵌套事务。
    pub(crate) fn reserve(&self, deadline: Instant) -> Result<(), PolicyError> {
        let connection = self.lock()?;
        connection
            .busy_timeout(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(250)),
            )
            .map_err(unavailable)?;
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(unavailable)
    }

    pub(crate) fn release(&self) -> Result<(), PolicyError> {
        // 即使源路径已被替换也必须释放原连接持有的锁。
        let connection = self
            .connection
            .lock()
            .map_err(|_| PolicyError::StoreUnavailable)?;
        if !connection.is_autocommit() {
            connection.execute_batch("ROLLBACK").map_err(unavailable)?;
        }
        Ok(())
    }

    pub(crate) fn wal_checkpoint(
        &self,
    ) -> Result<studiolo_core::checkpoint::WalCheckpointProof, PolicyError> {
        self.lock()?
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                Ok(studiolo_core::checkpoint::WalCheckpointProof {
                    busy: row.get(0)?,
                    log_frames: row.get(1)?,
                    checkpointed_frames: row.get(2)?,
                })
            })
            .map_err(unavailable)
    }

    pub(crate) fn vacuum_into(
        &self,
        target: &Storage,
        deadline: Instant,
    ) -> Result<(), PolicyError> {
        self.until(deadline, |connection| {
            if !connection.is_autocommit() {
                return Err(PolicyError::StoreUnavailable);
            }
            let path = target
                .database
                .to_str()
                .ok_or(PolicyError::CheckpointTarget)?;
            connection
                .execute("VACUUM INTO ?1", [path])
                .map_err(unavailable)?;
            Ok(())
        })
    }

    /// SQLite VM 内和操作返回后都核对截止时间；不遗留影响下一次操作的 hook。
    pub(crate) fn until<T>(
        &self,
        deadline: Instant,
        operation: impl FnOnce(&mut Connection) -> Result<T, PolicyError>,
    ) -> Result<T, PolicyError> {
        let mut connection = self.lock()?;
        if Instant::now() >= deadline {
            return Err(PolicyError::Busy);
        }
        connection
            .progress_handler(1000, Some(move || Instant::now() >= deadline))
            .map_err(unavailable)?;
        let result = operation(&mut connection);
        connection
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(unavailable)?;
        if Instant::now() >= deadline {
            return Err(PolicyError::Busy);
        }
        result
    }

    /// 写事务；闭包返回 `Err` 即整体回滚。
    pub(crate) fn write<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T, PolicyError>,
    ) -> Result<T, PolicyError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        let value = operation(&transaction)?;
        transaction.commit().map_err(unavailable)?;
        Ok(value)
    }

    /// 读事务：同一快照内读多张表。
    pub(crate) fn read<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T, PolicyError>,
    ) -> Result<T, PolicyError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(unavailable)?;
        let value = operation(&transaction)?;
        transaction.commit().map_err(unavailable)?;
        Ok(value)
    }
}

pub(crate) fn unavailable(error: rusqlite::Error) -> PolicyError {
    match error.sqlite_error_code() {
        Some(
            rusqlite::ErrorCode::DatabaseBusy
            | rusqlite::ErrorCode::DatabaseLocked
            | rusqlite::ErrorCode::OperationInterrupted,
        ) => PolicyError::Busy,
        _ => PolicyError::StoreUnavailable,
    }
}
