//! 策略私有库 `policy.sqlite3` 的唯一打开点；主人句柄与接入方句柄共用同一连接封装。
//!
//! 写事务一律 `BEGIN IMMEDIATE`，读事务用 DEFERRED 快照，使一次调用里读到的多张表彼此一致。

pub(crate) mod schema;

use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
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
        let storage = Storage::prepare(directory, policy)?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE;
        let mut connection =
            Connection::open_with_flags(&storage.database, flags).map_err(unavailable)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(unavailable)?;
        connection
            .execute_batch(
                "PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;",
            )
            .map_err(unavailable)?;
        {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(unavailable)?;
            schema::initialize(&transaction, storage.created)?;
            storage.check()?;
            transaction.commit().map_err(unavailable)?;
        }
        let mode: String = connection
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .map_err(unavailable)?;
        if mode != "wal" {
            return Err(PolicyError::StoreUnavailable);
        }
        storage.check()?;
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

pub(crate) fn unavailable(_: rusqlite::Error) -> PolicyError {
    PolicyError::StoreUnavailable
}
