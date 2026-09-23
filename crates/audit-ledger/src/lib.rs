//! 权威审计 Interface：精确幂等、追加顺序与完整性链。
//! 内存 Adapter 用于契约；SQLite Adapter 提供私有本地目录上的持久存储，并按固定边界分段，
//! 使重启验链与常规查询只付「段链 + 实际读到的段」的代价，而不是每次重扫整个账本。

mod codec;
mod event;
mod filter;
mod memory;
mod payload;
mod record;
mod sqlite;
mod verification;

pub use event::{AuditEvent, AuditEventId, AuditPhase, AuditSubject};
pub use filter::AuditFilter;
pub use memory::MemoryAuditLedger;
pub use payload::SanitizedPayload;
pub use record::{AuditRecord, IntegrityHash};
pub use sqlite::{AUDIT_SEGMENT_RECORDS, SqliteAuditLedger};
pub use verification::{AuditSegmentSummary, AuditVerificationCost};

/// 权威审计接缝上的错误。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AuditError {
    /// 事件标识为空、仅含空白或含控制字符。
    #[error("audit event id is invalid")]
    InvalidEventId,
    /// payload 中某个键可能携带机密，因此不能持久化。
    #[error("audit payload contains forbidden secret field at {path}")]
    SecretField {
        /// 被拒字段的路径：对象键以点号连接，数组元素记作 `[下标]`。
        path: String,
    },
    /// 既有幂等键（事件标识 + 阶段）被复用于不同的语义内容。
    #[error("audit event {event_id}/{phase:?} conflicts with an existing record")]
    IdempotencyConflict {
        /// 发生冲突的稳定事件标识。
        event_id: AuditEventId,
        /// 发生冲突的阶段。
        phase: AuditPhase,
    },
    /// 事件无法规范编码以供哈希与持久化，或编码结果不能无损回放。
    #[error("audit event serialization failed")]
    Serialization,
    /// 权威审计存储不可用。
    #[error("audit ledger is unavailable")]
    Unavailable,
    /// 只打开既有存储时，固定数据库文件不存在；须另行显式初始化。
    #[error("审计存储不存在；需要显式初始化")]
    MissingStore,
    /// 本地私有目录、权限或对象身份不满足存储约束。
    #[error("audit storage target is unsafe")]
    UnsafeStorage,
    /// 存储格式版本或编码版本未知，不能自动迁移或重置。
    #[error("audit storage schema is unsupported")]
    UnsupportedSchema,
    /// 数据库结构、连续 offset、事件或哈希链不一致。
    #[error("audit storage integrity verification failed")]
    CorruptStore,
    /// 当前平台未实现安全的私有本地文件约束。
    #[error("audit storage platform is unsupported")]
    UnsupportedPlatform,
    /// 单条规范编码超过持久 Adapter 的资源上限。
    #[error("audit record exceeds storage limit")]
    RecordTooLarge,
    /// payload 任一叶节点距离根超过 64 层，不能安全编码和回放。
    #[error("audit payload exceeds nesting limit")]
    PayloadTooDeep,
}

/// 权威审计持久化的同步边界；对象安全，可作 `dyn AuditLedger` 使用。
pub trait AuditLedger: Send + Sync {
    /// 追加一条事件；完全相同的重试直接返回既有记录。
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError>;

    /// 按追加顺序查询记录。
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError>;
}
