//! 告警与健康页的封闭参数类型；只有数字与角色标签，**不含任何地址或凭据**。

/// 死人开关的周期参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeadmanSchedule {
    pub(super) period_seconds: u64,
    pub(super) debounce_seconds: u64,
    pub(super) cooldown_seconds: u64,
    pub(super) max_suppressed: u32,
}

impl DeadmanSchedule {
    /// 约定活性回执周期。
    #[must_use]
    pub fn period_seconds(self) -> u64 {
        self.period_seconds
    }
    /// 超期后的去抖窗口。
    #[must_use]
    pub fn debounce_seconds(self) -> u64 {
        self.debounce_seconds
    }
    /// 两次告警之间的冷却。
    #[must_use]
    pub fn cooldown_seconds(self) -> u64 {
        self.cooldown_seconds
    }
    /// 冷却期内允许压制的最大次数；到顶必须强制升级再报。
    #[must_use]
    pub fn max_suppressed(self) -> u32 {
        self.max_suppressed
    }
}

/// 磁盘配额水位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskLimits {
    pub(super) minimum_free_bytes: u64,
    pub(super) warn_free_bytes: u64,
}

impl DiskLimits {
    /// 阻断下限：写完之后剩余空间不得低于此值。
    #[must_use]
    pub fn minimum_free_bytes(self) -> u64 {
        self.minimum_free_bytes
    }
    /// 预警水位。
    #[must_use]
    pub fn warn_free_bytes(self) -> u64 {
        self.warn_free_bytes
    }
}

/// Sync 范围配额。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncLimits {
    pub(super) max_objects: u64,
    pub(super) max_bytes: u64,
    pub(super) warn_objects: u64,
    pub(super) warn_bytes: u64,
}

impl SyncLimits {
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
    /// 对象数预警水位。
    #[must_use]
    pub fn warn_objects(self) -> u64 {
        self.warn_objects
    }
    /// 字节数预警水位。
    #[must_use]
    pub fn warn_bytes(self) -> u64 {
        self.warn_bytes
    }
}

/// 告警调度预算。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DispatchLimits {
    pub(super) max_attempts: u32,
    pub(super) dedupe_capacity: u32,
}

impl DispatchLimits {
    /// 单轮最大尝试次数；没有"无限重试"。
    #[must_use]
    pub fn max_attempts(self) -> u32 {
        self.max_attempts
    }
    /// 幂等键去重表容量。
    #[must_use]
    pub fn dedupe_capacity(self) -> u32 {
        self.dedupe_capacity
    }
}
