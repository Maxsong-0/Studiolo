//! 控制面持久化失败的统一错误类型。
use crate::AttemptPhase;
use studiolo_core::VaultUuid;

/// 控制面持久化失败。
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// SQLite 拒绝了某个操作。
    #[error("control database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// JSON 序列化或解析失败。
    #[error("control database JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// Vault 绑定不一致：数据库已绑定到另一个 Vault，或输入快照属于另一个 Vault。
    #[error("control database is bound to {expected}, not {actual}")]
    VaultBindingMismatch {
        /// 已持久化的绑定。
        expected: VaultUuid,
        /// 请求方（或输入快照）给出的 Vault。
        actual: VaultUuid,
    },
    /// 引用的记录不存在。
    #[error("control record not found: {0}")]
    NotFound(&'static str),
    /// 从当前持久状态出发不允许此迁移。
    #[error("invalid transition from {from} via {operation}")]
    InvalidTransition {
        /// 当前状态（入参不合法时为说明文字或被拒的目标状态）。
        from: String,
        /// 请求的操作。
        operation: &'static str,
    },
    /// 已有终态赢家（含已冻结提交计划的 committing）；属正常业务拒绝，调用方按变体而非状态名判断。
    #[error("attempt already holds a terminal outcome ({phase:?}) for {operation}")]
    AlreadyTerminal {
        /// 持有终态结果的持久阶段。
        phase: AttemptPhase,
        /// 被拒绝的操作名。
        operation: &'static str,
    },
    /// 数据库由更新版本的 schema 写入，本二进制不得静默降级运行。
    #[error("control db schema version {found} is newer than supported version {supported}")]
    UnsupportedSchema {
        /// `schema_migrations` 中记录的最高版本。
        found: i64,
        /// 本二进制支持的最高版本。
        supported: i64,
    },
    /// attempt 身份或 fence 不匹配（含已不在 running 或租约已过期）。
    #[error("attempt fence mismatch")]
    FenceMismatch,
    /// 活动工作已占有一个或多个请求的路径。
    #[error("paths held by {by_instance}: {paths:?}")]
    PathsHeld {
        #[doc = "持有冲突认领的实例。"]
        by_instance: String,
        #[doc = "与活动认领相交的请求路径。"]
        paths: Vec<String>,
    },
    /// 可机器校验的黑板规则拒绝了该操作。
    #[error("blackboard rejected: {0}")]
    BoardRejected(&'static str),
    /// 同一稳定请求 ID 被用于不同的输入。
    #[error("idempotency request mismatch")]
    IdempotencyMismatch,
    /// 持久值损坏或不一致（个别入参拒绝也归此类，如非白名单的 pragma 查询）。
    #[error("corrupt control record: {0}")]
    Corrupt(String),
}
