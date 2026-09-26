//! 单写者句柄：打开连接、配置连接级 PRAGMA、绑定 Vault 与只读诊断。
use crate::{DbError, schema_guard, types::parse_vault_uuid};
use rusqlite::{Connection, OptionalExtension as _, TransactionBehavior};
use std::path::Path;
use studiolo_core::VaultUuid;

/// 不可克隆、单写者的运行时权威。
pub struct ControlDb {
    pub(super) connection: Connection,
    pub(super) vault_uuid: VaultUuid,
}

impl ControlDb {
    /// 打开文件数据库：配置连接、执行迁移并绑定 Vault。
    pub fn open(path: impl AsRef<Path>, vault_uuid: VaultUuid) -> Result<Self, DbError> {
        Self::from_connection(Connection::open(path)?, vault_uuid)
    }

    /// 打开测试用内存数据库，走与文件库相同的配置、迁移与绑定流程。
    pub fn open_in_memory(vault_uuid: VaultUuid) -> Result<Self, DbError> {
        Self::from_connection(Connection::open_in_memory()?, vault_uuid)
    }

    fn from_connection(connection: Connection, vault_uuid: VaultUuid) -> Result<Self, DbError> {
        // 这些设置只影响当前连接；新版库判定前不切换持久 journal mode。
        connection.execute_batch(
            "PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;
             PRAGMA trusted_schema=OFF;",
        )?;
        schema_guard::require_supported_version(&connection)?;
        let mut db = Self {
            connection,
            vault_uuid,
        };
        db.migrate()?;
        db.connection.execute_batch("PRAGMA journal_mode=WAL;")?;
        db.bind_vault(vault_uuid)?;
        Ok(db)
    }

    fn bind_vault(&mut self, requested: VaultUuid) -> Result<(), DbError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored: Option<String> = tx
            .query_row(
                "SELECT vault_uuid FROM vault_binding WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match stored {
            None => {
                tx.execute(
                    "INSERT INTO vault_binding(singleton, vault_uuid) VALUES (1, ?1)",
                    [requested.to_string()],
                )?;
            }
            Some(value) => {
                let expected = parse_vault_uuid(&value)?;
                if expected != requested {
                    return Err(DbError::VaultBindingMismatch {
                        expected,
                        actual: requested,
                    });
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 返回已应用的最新 schema 版本。
    pub fn schema_version(&self) -> Result<i64, DbError> {
        Ok(self.connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?)
    }

    /// 返回持久化的 Vault 绑定。
    pub fn bound_vault_uuid(&self) -> Result<VaultUuid, DbError> {
        let value: String = self.connection.query_row(
            "SELECT vault_uuid FROM vault_binding WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        parse_vault_uuid(&value)
    }

    /// 读取白名单内的运行时 pragma，用于诊断。
    pub fn pragma(&self, name: &str) -> Result<String, DbError> {
        if !matches!(name, "journal_mode" | "synchronous" | "foreign_keys") {
            return Err(DbError::Corrupt("unsupported pragma query".to_owned()));
        }
        let value: rusqlite::types::Value =
            self.connection
                .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))?;
        match value {
            rusqlite::types::Value::Integer(value) => Ok(value.to_string()),
            rusqlite::types::Value::Text(value) => Ok(value),
            _ => Err(DbError::Corrupt("unexpected pragma value".to_owned())),
        }
    }
}
