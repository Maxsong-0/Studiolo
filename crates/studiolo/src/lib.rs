//! Studiolo：自托管的知识与项目工作台。
//!
//! M0 组合根装配文件系统 Broker、SQLite TaskEngine 和隔离 Stub。
//! 尚不启动常驻服务、网络入口或真实外部工具。

use std::path::Path;

use studiolo_audit_ledger::MemoryAuditLedger;
use studiolo_control_db::{ControlDb, DbError};
use studiolo_runner_adapter::StubRunner;
use studiolo_task_engine::PersistentTaskEngine;
use studiolo_vault_broker::{BrokerError, FsVaultBroker, VaultReader as _};

/// M0 使用持久运行状态及文件系统产物，审计依然使用内存 Adapter。
pub type M0TaskEngine = PersistentTaskEngine<FsVaultBroker, MemoryAuditLedger>;

/// 装配失败的来源；不会隐藏 Vault 身份失配等错误。
#[derive(Debug, thiserror::Error)]
pub enum M0OpenError {
    /// Broker 初始化失败。
    #[error("初始化 Vault 失败：{0}")]
    Broker(#[from] BrokerError),
    /// Control DB 初始化或 Vault 绑定失败。
    #[error("初始化运行数据库失败：{0}")]
    ControlDb(#[from] DbError),
}

/// 无常驻进程的 M0 组合，所有任务写操作由 TaskEngine 持有。
pub struct M0Workbench {
    /// 独占 Control DB 与 Broker 写能力的任务引擎。
    pub tasks: M0TaskEngine,
    /// 仅写私有临时目录的同步 Stub。
    pub runner: StubRunner,
}

impl M0Workbench {
    /// 打开一个 Vault 及其配套的数据目录；不会自动恢复或重新运行旧任务。
    pub fn open(vault_root: &Path, data_root: &Path) -> Result<Self, M0OpenError> {
        let broker = FsVaultBroker::open(vault_root, data_root)?;
        let db = ControlDb::open(data_root.join("control.db"), broker.vault_uuid())?;
        Ok(Self {
            tasks: PersistentTaskEngine::new(db, broker, MemoryAuditLedger::new()),
            runner: StubRunner::new(),
        })
    }
}

/// 返回工作区版本；crates.io 上的占位包与本地 M0 实现分别管理。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_not_empty() {
        assert!(!super::version().is_empty());
    }
}
