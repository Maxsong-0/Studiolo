//! 持久化的 M0 适配器 `PersistentTaskEngine` 本体：构造、只读访问与部件移交。
use super::*;

/// 持久化的 M0 适配器；独占持有 Control DB 与 Broker 写入端。
pub struct PersistentTaskEngine<B, L> {
    // 三个部件放宽到 crate 内可见：各职责模块（含 board、lifecycle 等）的 impl 块直接取用。
    pub(super) control_db: ControlDb,
    pub(super) broker: B,
    pub(super) audit_ledger: L,
}

impl<B, L> PersistentTaskEngine<B, L> {
    /// 接管 Control DB 与 Broker 写入端的独占所有权。
    pub fn new(control_db: ControlDb, broker: B, audit_ledger: L) -> Self {
        Self {
            control_db,
            broker,
            audit_ledger,
        }
    }

    /// 只读访问，供组合与重启断言使用。
    #[must_use]
    pub fn control_db(&self) -> &ControlDb {
        &self.control_db
    }

    /// 只读访问所持有的 Broker 适配器。
    #[must_use]
    pub fn broker(&self) -> &B {
        &self.broker
    }

    /// 只读访问所持有的审计适配器。
    #[must_use]
    pub fn audit_ledger(&self) -> &L {
        &self.audit_ledger
    }

    /// 交出所持有的各部件，供组合根显式移交。
    #[must_use]
    pub fn into_parts(self) -> (ControlDb, B, L) {
        (self.control_db, self.broker, self.audit_ledger)
    }
}
