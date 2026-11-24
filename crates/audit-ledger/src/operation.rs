//! 跨进程完整操作的生命周期；仅 SQLite 存储适配器能构造持久许可。

use std::fs::File;
use std::time::Duration;

/// 等不到许可时隔多久再试一次。封存先等人侧空闲时也按它看（BUG-056）：排空看得到的空当，那里同样看得到——
/// 操作密、空当短的时候，不会因为先等那一步反而等不到。
pub const OPERATION_RETRY: Duration = Duration::from_millis(5);

/// 一次完整语义操作的共享许可。可跨线程移动，丢弃即释放。
/// 不提供克隆，以免请求结束后另留无法追踪的许可。
pub struct OperationPermit {
    pub(crate) _lease: Option<Lease>,
}

impl OperationPermit {
    pub(crate) fn uncoordinated() -> Self {
        Self { _lease: None }
    }
}

/// 同一权威审计根上的全部协作操作已排空，并阻止新操作进入。
/// 这不是 SQLite 写锁；仍须按固定 owner 顺序冻结各存储并核对 pending。
pub struct AuditQuiescence {
    pub(crate) _lease: Lease,
}

pub(crate) struct Lease(pub(crate) File);

impl Drop for Lease {
    fn drop(&mut self) {
        // 显式解锁也避免 fork 后、exec 前的短暂 FD 继承延长窗口。
        #[cfg(unix)]
        let _ = rustix::fs::flock(&self.0, rustix::fs::FlockOperation::Unlock);
    }
}
