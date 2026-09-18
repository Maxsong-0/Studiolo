//! 磁盘剩余空间配额：显式最低剩余与预警剩余，超限阻断写入。

use super::{Admission, BlockReason, QuotaError, QuotaVerdict};

/// 一次磁盘读数。字节数由调用方从真实设备取得，本模块不做 IO。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskReading {
    /// 该设备当前剩余可用字节数。
    pub free_bytes: u64,
}

/// 磁盘配额：低于 `warn_free_bytes` 预警，低于 `minimum_free_bytes` 阻断。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskQuota {
    minimum_free_bytes: u64,
    warn_free_bytes: u64,
}

impl DiskQuota {
    /// 两个水位都必须显式给出且大于 0，预警水位不得比阻断下限更紧。
    pub fn new(minimum_free_bytes: u64, warn_free_bytes: u64) -> Result<Self, QuotaError> {
        if minimum_free_bytes == 0 || warn_free_bytes < minimum_free_bytes {
            return Err(QuotaError::InvalidLimits);
        }
        Ok(Self {
            minimum_free_bytes,
            warn_free_bytes,
        })
    }

    /// 阻断下限。
    #[must_use]
    pub fn minimum_free_bytes(self) -> u64 {
        self.minimum_free_bytes
    }

    /// 预警水位。
    #[must_use]
    pub fn warn_free_bytes(self) -> u64 {
        self.warn_free_bytes
    }

    /// 对当前读数下结论；`None` 读数即 [`QuotaVerdict::Unknown`]，不算正常。
    #[must_use]
    pub fn evaluate(self, reading: Option<DiskReading>) -> QuotaVerdict {
        let Some(reading) = reading else {
            return QuotaVerdict::Unknown;
        };
        if reading.free_bytes < self.minimum_free_bytes {
            QuotaVerdict::Exceeded
        } else if reading.free_bytes < self.warn_free_bytes {
            QuotaVerdict::Warning
        } else {
            QuotaVerdict::Normal
        }
    }

    /// 写入前准入：写完之后剩余空间仍须 ≥ 下限；等号可过。
    ///
    /// 读数缺失不得放行——没有证据就当作不满足下限，按阻断处理。
    pub fn admit(
        self,
        reading: Option<DiskReading>,
        write_bytes: u64,
    ) -> Result<Admission, QuotaError> {
        let Some(reading) = reading else {
            return Ok(Admission::Blocked(BlockReason::DiskFreeBelowMinimum));
        };
        let remaining = reading
            .free_bytes
            .checked_sub(write_bytes)
            .ok_or(QuotaError::ArithmeticOverflow)?;
        if remaining >= self.minimum_free_bytes {
            Ok(Admission::Allowed)
        } else {
            Ok(Admission::Blocked(BlockReason::DiskFreeBelowMinimum))
        }
    }
}
