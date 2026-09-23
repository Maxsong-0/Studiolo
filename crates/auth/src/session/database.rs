//! 唯一 SQLite 打开点；事务仅执行本库逻辑，不调用 AuditLedger。

use super::{
    SessionConfig, SessionError, SessionRandom, schema, secrets,
    storage::{CreatePolicy, Storage},
};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::{path::Path, sync::Mutex, time::Duration};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Migration {
    None,
    SessionV1,
    TotpV2,
    RecoveryV3,
}

pub(super) struct Database {
    pub(super) connection: Mutex<Connection>,
    storage: Storage,
    config: String,
}

impl Database {
    pub(super) fn open(
        root: &Path,
        config: &SessionConfig,
        random: &dyn SessionRandom,
        migrate: Migration,
        create: CreatePolicy,
    ) -> Result<Self, SessionError> {
        let storage = Storage::prepare(root, create)?;
        let store_id = if storage.created {
            let mut bytes = [0_u8; 16];
            random.fill(&mut bytes)?;
            Some(secrets::hex(&bytes))
        } else {
            None
        };
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE;
        let mut connection =
            Connection::open_with_flags(&storage.database, flags).map_err(unavailable)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(unavailable)?;
        connection.execute_batch("PRAGMA trusted_schema=OFF;PRAGMA foreign_keys=ON;PRAGMA synchronous=FULL;PRAGMA fullfsync=ON;PRAGMA checkpoint_fullfsync=ON;").map_err(unavailable)?;
        configuration(&connection)?;
        let config = config.encoded()?;
        {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(unavailable)?;
            schema::initialize(
                &transaction,
                storage.created,
                &config,
                store_id.as_deref(),
                migrate,
            )?;
            storage.check()?;
            transaction.commit().map_err(unavailable)?;
        }
        let mode: String = connection
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            .map_err(unavailable)?;
        if mode != "wal" {
            return Err(SessionError::StorageUnavailable);
        }
        configuration(&connection)?;
        storage.check()?;
        Ok(Self {
            connection: Mutex::new(connection),
            storage,
            config,
        })
    }

    pub(super) fn transaction<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, SessionError>,
    ) -> Result<T, SessionError> {
        self.storage.check()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| SessionError::StorageUnavailable)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        if !matches!(schema::version(&transaction)?, 2..=4) {
            return Err(SessionError::UnsupportedSchema);
        }
        schema::validate(&transaction, &self.config)?;
        let value = operation(&transaction)?;
        self.storage.check()?;
        transaction.commit().map_err(unavailable)?;
        self.storage.check()?;
        Ok(value)
    }
}

pub(super) fn unavailable(_: rusqlite::Error) -> SessionError {
    SessionError::StorageUnavailable
}

/// 按不变量必然存在的行（单例行、成对或被引用的关联行）专用：查不到即库已损坏；
/// 锁、IO 与其他读取错误仍是存储暂不可用。业务上可缺的行应走 `.optional()`。
pub(super) fn required_row(error: rusqlite::Error) -> SessionError {
    match error {
        rusqlite::Error::QueryReturnedNoRows => SessionError::CorruptStore,
        _ => SessionError::StorageUnavailable,
    }
}

fn configuration(db: &Connection) -> Result<(), SessionError> {
    for (pragma, expected) in [
        ("PRAGMA synchronous", 2),
        ("PRAGMA fullfsync", 1),
        ("PRAGMA checkpoint_fullfsync", 1),
        ("PRAGMA foreign_keys", 1),
        ("PRAGMA trusted_schema", 0),
        ("PRAGMA busy_timeout", 5000),
    ] {
        let value: i64 = db
            .query_row(pragma, [], |r| r.get(0))
            .map_err(unavailable)?;
        if value != expected {
            return Err(SessionError::StorageUnavailable);
        }
    }
    Ok(())
}
