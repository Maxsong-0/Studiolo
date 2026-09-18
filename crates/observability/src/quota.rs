//! 磁盘与 Sync 配额：显式上限、显式预警水位，超限阻断写入而不是只告警。
//!
//! 所有整数运算一律 `checked_*`；任何溢出都判为拒绝，绝不回绕后放行。
//! 本模块没有 IO，也不读取真实磁盘——读数由调用方注入，便于对等号、
//! 溢出与证据缺失逐一写契约。

mod disk;
mod sync;

pub use disk::{DiskQuota, DiskReading};
pub use sync::{SyncQuota, SyncUsage};

/// 配额判定失败的固定分类；不回显具体数值，避免把容量画像带出边界。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QuotaError {
    /// 上限与预警水位本身不自洽（为 0、预警比上限更宽等）。
    #[error("配额上限或预警水位不自洽")]
    InvalidLimits,
    /// 参与判定的整数越界；一律拒绝而不是回绕。
    #[error("配额判定的整数运算越界")]
    ArithmeticOverflow,
}

/// 配额结论。没有读数时只能是 [`QuotaVerdict::Unknown`]。
///
/// 默认值刻意是 [`QuotaVerdict::Unknown`]：任何"忘了填"的路径都必须落到未知，
/// 绝不能因为 `Default` 而被悄悄当成正常。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QuotaVerdict {
    /// 在预警水位之内。
    Normal,
    /// 已越过预警水位但尚未越过上限。
    Warning,
    /// 已越过上限；对应写入路径必须阻断。
    Exceeded,
    /// 读数缺失或不可信。
    #[default]
    Unknown,
}

impl QuotaVerdict {
    /// 映射到健康三态。预警与超限都不是绿。
    #[must_use]
    pub fn health(self) -> crate::health::HealthState {
        match self {
            Self::Normal => crate::health::HealthState::Normal,
            Self::Warning | Self::Exceeded => crate::health::HealthState::Degraded,
            Self::Unknown => crate::health::HealthState::Unknown,
        }
    }
}

/// 写入准入结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// 本次写入可以放行。
    Allowed,
    /// 本次写入必须阻断，附固定原因分类。
    Blocked(BlockReason),
}

impl Admission {
    /// 是否放行。
    #[must_use]
    pub fn allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }
}

/// 阻断原因的固定分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    /// 写入后设备剩余空间会低于约定下限。
    DiskFreeBelowMinimum,
    /// 写入后 Sync 范围内对象数会超过上限。
    SyncObjectLimit,
    /// 写入后 Sync 范围内字节数会超过上限。
    SyncByteLimit,
}
