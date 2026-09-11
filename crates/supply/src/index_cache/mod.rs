//! 可删除重建的派生索引缓存；不保存身份、凭据或审计事实，也不决定供给权限。

mod paths;
mod records;
mod schema;

use std::{path::Path, sync::Mutex, time::Duration};

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use sha2::{Digest as _, Sha256};

use crate::SupplyError;
use paths::Storage;

/// 单批编码索引的硬上限；读侧在申请负载内存之前检查长度。
pub const MAX_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;
use records::{read_payload, validate_scope};
use schema::{APPLICATION_ID, SCHEMA_VERSION, TABLE_SQL, validate_schema};

/// 固定 `index.sqlite3` 的单槽缓存；换来源或白名单只保留最新一批。
pub struct IndexCacheStore {
    connection: Mutex<Connection>,
    storage: Storage,
}

/// 创建库之前核对缓存与源、身份及审计根隔离；只读检查，不创建目录或文件。
pub fn validate_isolation(root: &Path, forbidden: &[&Path]) -> Result<(), SupplyError> {
    paths::validate_isolation(root, forbidden)
}

impl IndexCacheStore {
    /// 打开已有绝对 0700 目录，在其中创建 0600 缓存；损坏或未知库不自动改写。
    pub fn open(root: &Path) -> Result<Self, SupplyError> {
        let (storage, created) = Storage::prepare(root)?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE;
        let opened = Connection::open_with_flags(&storage.database, flags).map_err(database_error);
        storage.check()?;
        let mut connection = opened?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(database_error)?;
        connection
            .execute_batch(
                "PRAGMA trusted_schema=OFF; PRAGMA secure_delete=ON;
             PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY;",
            )
            .map_err(database_error)?;
        if created {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database_error)?;
            transaction
                .execute_batch(TABLE_SQL)
                .map_err(database_error)?;
            transaction
                .pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(database_error)?;
            transaction
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(database_error)?;
            transaction.commit().map_err(database_error)?;
        }
        let checked = validate_schema(&connection);
        storage.check()?;
        checked?;
        Ok(Self {
            connection: Mutex::new(connection),
            storage,
        })
    }

    /// 加载同一命名空间的完整字节；来源或白名单摘要不同视为未命中。
    pub fn load(&self, scope: &str) -> Result<Option<Vec<u8>>, SupplyError> {
        validate_scope(scope)?;
        self.checked(|connection| {
            let transaction = connection.transaction().map_err(database_error)?;
            validate_schema(&transaction)?;
            let payload = read_payload(&transaction)?;
            transaction.commit().map_err(database_error)?;
            Ok(payload
                .filter(|(stored_scope, _)| stored_scope == scope)
                .map(|(_, payload)| payload))
        })
    }

    /// 原子替换唯一缓存批次；失败保留上一批，不累计历史来源资料。
    pub fn replace(&self, scope: &str, payload: &[u8]) -> Result<(), SupplyError> {
        validate_scope(scope)?;
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(SupplyError::CorruptStore);
        }
        self.checked(|connection| {
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database_error)?;
            validate_schema(&transaction)?;
            read_payload(&transaction)?;
            transaction.execute(
                "INSERT INTO cache_entry(slot, scope, payload, bytes, sha256) VALUES(1, ?1, ?2, ?3, ?4)
                 ON CONFLICT(slot) DO UPDATE SET scope=excluded.scope, payload=excluded.payload,
                    bytes=excluded.bytes, sha256=excluded.sha256",
                params![scope, payload, payload.len() as i64, Sha256::digest(payload).as_slice()],
            ).map_err(database_error)?;
            transaction.commit().map_err(database_error)
        })
    }

    fn checked<T>(
        &self,
        action: impl FnOnce(&mut Connection) -> Result<T, SupplyError>,
    ) -> Result<T, SupplyError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| SupplyError::StoreUnavailable)?;
        self.storage.check()?;
        let result = action(&mut connection);
        self.storage.check()?;
        result
    }
}

pub(super) fn database_error(error: rusqlite::Error) -> SupplyError {
    match error {
        rusqlite::Error::SqliteFailure(error, _)
            if matches!(
                error.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            ) =>
        {
            SupplyError::CorruptStore
        }
        _ => SupplyError::StoreUnavailable,
    }
}
