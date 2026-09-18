//! Sync 范围配额：范围内对象数与字节数各有显式上限与预警水位。

use super::{Admission, BlockReason, QuotaError, QuotaVerdict};

/// Sync 范围当前占用。由 Bridge/Broker 自己统计后注入，本模块不扫描。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncUsage {
    /// 范围内对象数。
    pub objects: u64,
    /// 范围内逻辑字节数。
    pub bytes: u64,
}

/// Sync 配额；两条维度各自独立判定，任一超限都阻断。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncQuota {
    max_objects: u64,
    max_bytes: u64,
    warn_objects: u64,
    warn_bytes: u64,
}

impl SyncQuota {
    /// 四个数都必须显式给出且大于 0，预警水位不得超过对应上限。
    pub fn new(
        max_objects: u64,
        max_bytes: u64,
        warn_objects: u64,
        warn_bytes: u64,
    ) -> Result<Self, QuotaError> {
        if max_objects == 0
            || max_bytes == 0
            || warn_objects == 0
            || warn_bytes == 0
            || warn_objects > max_objects
            || warn_bytes > max_bytes
        {
            return Err(QuotaError::InvalidLimits);
        }
        Ok(Self {
            max_objects,
            max_bytes,
            warn_objects,
            warn_bytes,
        })
    }

    /// 对象数上限。
    #[must_use]
    pub fn max_objects(self) -> u64 {
        self.max_objects
    }

    /// 字节数上限。
    #[must_use]
    pub fn max_bytes(self) -> u64 {
        self.max_bytes
    }

    /// 对当前占用下结论；`None` 即未知，不算正常。
    #[must_use]
    pub fn evaluate(self, usage: Option<SyncUsage>) -> QuotaVerdict {
        let Some(usage) = usage else {
            return QuotaVerdict::Unknown;
        };
        if usage.objects > self.max_objects || usage.bytes > self.max_bytes {
            QuotaVerdict::Exceeded
        } else if usage.objects >= self.warn_objects || usage.bytes >= self.warn_bytes {
            QuotaVerdict::Warning
        } else {
            QuotaVerdict::Normal
        }
    }

    /// 写入前准入：加上本次增量后两条维度都须 ≤ 上限；等号可过。
    ///
    /// 占用证据缺失不得放行；加法溢出一律 [`QuotaError::ArithmeticOverflow`]。
    pub fn admit(
        self,
        usage: Option<SyncUsage>,
        added_objects: u64,
        added_bytes: u64,
    ) -> Result<Admission, QuotaError> {
        let Some(usage) = usage else {
            return Ok(Admission::Blocked(BlockReason::SyncObjectLimit));
        };
        let objects = usage
            .objects
            .checked_add(added_objects)
            .ok_or(QuotaError::ArithmeticOverflow)?;
        let bytes = usage
            .bytes
            .checked_add(added_bytes)
            .ok_or(QuotaError::ArithmeticOverflow)?;
        if objects > self.max_objects {
            return Ok(Admission::Blocked(BlockReason::SyncObjectLimit));
        }
        if bytes > self.max_bytes {
            return Ok(Admission::Blocked(BlockReason::SyncByteLimit));
        }
        Ok(Admission::Allowed)
    }
}
