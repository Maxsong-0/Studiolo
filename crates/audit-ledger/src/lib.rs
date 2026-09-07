//! 权威审计 Interface：精确幂等、追加顺序与完整性链。
//! 内存 Adapter 用于契约；SQLite Adapter 提供私有本地目录上的持久存储。

mod codec;
mod event;
mod filter;
mod memory;
mod payload;
mod record;
mod sqlite;

pub use event::{AuditEvent, AuditEventId, AuditPhase, AuditSubject};
pub use filter::AuditFilter;
pub use memory::MemoryAuditLedger;
pub use payload::SanitizedPayload;
pub use record::{AuditRecord, IntegrityHash};
pub use sqlite::SqliteAuditLedger;

/// Errors at the authoritative audit seam.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AuditError {
    /// An event identifier was empty or contained control characters.
    #[error("audit event id is invalid")]
    InvalidEventId,
    /// A payload key could carry a secret and therefore cannot be persisted.
    #[error("audit payload contains forbidden secret field at {path}")]
    SecretField {
        /// Dot-and-index path of the rejected field.
        path: String,
    },
    /// An existing idempotency key was reused for different semantic content.
    #[error("audit event {event_id}/{phase:?} conflicts with an existing record")]
    IdempotencyConflict {
        /// Conflicting stable identifier.
        event_id: AuditEventId,
        /// Conflicting phase.
        phase: AuditPhase,
    },
    /// The event could not be encoded for hashing and persistence.
    #[error("audit event serialization failed")]
    Serialization,
    /// The authoritative audit store is unavailable.
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

/// Synchronous, object-safe boundary for authoritative audit persistence.
pub trait AuditLedger: Send + Sync {
    /// Appends an event or returns the existing record for an identical retry.
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError>;

    /// Queries records in append order.
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError>;
}
