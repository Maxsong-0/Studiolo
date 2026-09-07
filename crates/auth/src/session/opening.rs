//! 打开与显式迁移的装配；普通不创建打开从存储层到 SQLite 均禁止补建主库。

use super::{
    SessionClock, SessionConfig, SessionError, SessionManager, SessionRandom,
    database::{Database, Migration},
    storage::CreatePolicy,
};
use std::{path::Path, sync::Arc};
use studiolo_audit_ledger::AuditLedger;

impl SessionManager {
    /// 打开预先批准、位于 Vault/Audit/Control 存储之外的私有绝对目录。
    ///
    /// Unix 0700/0600、本地锁与可信父目录是前置条件；不创建目录、不迁移未知配置。
    pub fn open(
        root: &Path,
        config: SessionConfig,
        audit: Arc<dyn AuditLedger>,
        clock: Arc<dyn SessionClock>,
        random: Arc<dyn SessionRandom>,
    ) -> Result<Self, SessionError> {
        Self::open_mode(
            root,
            config,
            audit,
            clock,
            random,
            Migration::None,
            CreatePolicy::Allow,
        )
    }

    /// 仅打开已有固定 `auth.sqlite3`；缺失主库返回 `MissingStore`，绝不创建空库。
    ///
    /// 保留 v2/v3/v4 的格式、主人、权限、回执与时钟检查；v1 仍须显式迁移。
    /// 这不是只读数据库句柄：合法打开仍可更新时钟/到期状态并创建 SQLite 侧文件。
    pub fn open_existing(
        root: &Path,
        config: SessionConfig,
        audit: Arc<dyn AuditLedger>,
        clock: Arc<dyn SessionClock>,
        random: Arc<dyn SessionRandom>,
    ) -> Result<Self, SessionError> {
        Self::open_mode(
            root,
            config,
            audit,
            clock,
            random,
            Migration::None,
            CreatePolicy::ExistingOnly,
        )
    }

    /// 显式验证并原子升级 v1 存储；已有 v2 可幂等打开，不改绑主人或重新授权。
    ///
    /// 迁移后旧二进制必须停止写入；旧活跃凭证及原有效期保持不变。
    pub fn migrate_v1(
        root: &Path,
        config: SessionConfig,
        audit: Arc<dyn AuditLedger>,
        clock: Arc<dyn SessionClock>,
        random: Arc<dyn SessionRandom>,
    ) -> Result<Self, SessionError> {
        Self::open_mode(
            root,
            config,
            audit,
            clock,
            random,
            Migration::SessionV1,
            CreatePolicy::Allow,
        )
    }

    /// 显式将已验证的 v2 增量升级为 v3 因子存储，不登记因子或赋予任何会话 L3。
    ///
    /// 新目录先建立 v2 再同事务增设 v3；v1 须先调用 `migrate_v1`，未知/损坏格式拒绝。
    pub fn migrate_v2(
        root: &Path,
        config: SessionConfig,
        audit: Arc<dyn AuditLedger>,
        clock: Arc<dyn SessionClock>,
        random: Arc<dyn SessionRandom>,
    ) -> Result<Self, SessionError> {
        Self::open_mode(
            root,
            config,
            audit,
            clock,
            random,
            Migration::TotpV2,
            CreatePolicy::Allow,
        )
    }

    /// 显式将已验证的 v3 增量升级为 v4 恢复存储，不生成恢复码、登记或替换因子。
    ///
    /// 新目录在同一事务中依次建立 v1～v4；已有 v1/v2 须先经原迁移入口升级。
    /// 已有合法 v4 可幂等打开；未知或损坏格式拒绝，不自动修复或清空数据。
    pub fn migrate_v3(
        root: &Path,
        config: SessionConfig,
        audit: Arc<dyn AuditLedger>,
        clock: Arc<dyn SessionClock>,
        random: Arc<dyn SessionRandom>,
    ) -> Result<Self, SessionError> {
        Self::open_mode(
            root,
            config,
            audit,
            clock,
            random,
            Migration::RecoveryV3,
            CreatePolicy::Allow,
        )
    }

    fn open_mode(
        root: &Path,
        config: SessionConfig,
        audit: Arc<dyn AuditLedger>,
        clock: Arc<dyn SessionClock>,
        random: Arc<dyn SessionRandom>,
        migrate: Migration,
        create: CreatePolicy,
    ) -> Result<Self, SessionError> {
        let database = Database::open(root, &config, random.as_ref(), migrate, create)?;
        let manager = Self {
            database,
            config,
            audit,
            clock,
            random,
        };
        manager.run(|_, _| Ok(Ok(())))?;
        Ok(manager)
    }
}
