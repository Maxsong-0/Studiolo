//! 秘密驻留面的固定分类：每项措施只能是「已生效并核读」或「本平台不提供」。
//!
//! 分类是封闭枚举而不是布尔或字符串，任何平台差异都必须在此显式建模；
//! 没有「尽力而为」「大概生效」的取值，也没有让缺失能力沉默通过的默认分支。
//!
//! 平台差异建模成**两套互斥的取值**而不是同一个取值加 `#[cfg]` 改写含义：
//! 只有 Linux 能构造 [`Applied::NotDumpable`]，只有非 Linux 能构造
//! [`Unsupported::NoDumpableSwitch`]，编译器因此不允许把缺失能力说成已生效。

use std::fmt;

/// 读取秘密之前收紧的单项措施；数组顺序即执行顺序。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Measure {
    /// 核心转储软硬限额降为 0，并回读确认。
    CoreDumpLimit,
    /// 关闭本进程的可转储属性：同时挡住外部 ptrace 与同 UID 读取本进程的
    /// `/proc/<pid>/environ`、`/proc/<pid>/mem`、`/proc/<pid>/maps`。
    ProcessInspection,
    /// 把已映射与后续映射标记为锁定，秘密页不被换出到交换区。
    MemoryResidency,
    /// 经固定环境槽位传入的种子在本进程内的驻留处置。
    EnvironmentSlot,
}

impl Measure {
    /// 措施集合是封闭的；新增一项必须同时补齐 `ORDER` 与 `index`。
    pub(crate) const COUNT: usize = 4;

    /// 固定执行顺序：先挡崩溃取证，再挡在线读取，最后挡换出。
    pub(crate) const ORDER: [Self; Self::COUNT] = [
        Self::CoreDumpLimit,
        Self::ProcessInspection,
        Self::MemoryResidency,
        Self::EnvironmentSlot,
    ];

    /// 在固定结果数组中的下标；编译器强制这里覆盖全部取值。
    #[cfg(test)]
    pub(crate) fn index(self) -> usize {
        match self {
            Self::CoreDumpLimit => 0,
            Self::ProcessInspection => 1,
            Self::MemoryResidency => 2,
            Self::EnvironmentSlot => 3,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::CoreDumpLimit => "核心转储限额",
            Self::ProcessInspection => "进程可检视属性",
            Self::MemoryResidency => "秘密页驻留",
            Self::EnvironmentSlot => "环境槽位",
        }
    }
}

impl fmt::Display for Measure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// 措施已经在本进程生效，并且已按本模块声明的方式回读确认。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    /// 软硬限额均为 0，且回读为 0。
    CoreDumpLimitZeroed,
    /// `PR_SET_DUMPABLE(0)` 已生效，且 `PR_GET_DUMPABLE` 回读为不可转储。
    #[cfg(target_os = "linux")]
    NotDumpable,
    /// `mlockall` 把全部映射标记为「缺页时锁定」，只锁真正触碰过的页。
    #[cfg(target_os = "linux")]
    LockedOnFault,
    /// 内核不支持「缺页时锁定」，退回整地址空间预先锁定。
    #[cfg(target_os = "linux")]
    LockedEagerly,
}

impl Applied {
    fn label(self) -> &'static str {
        match self {
            Self::CoreDumpLimitZeroed => "软硬限额已降为 0 并回读确认",
            #[cfg(target_os = "linux")]
            Self::NotDumpable => "已置为不可转储并回读确认",
            #[cfg(target_os = "linux")]
            Self::LockedOnFault => "全部映射已按缺页锁定",
            #[cfg(target_os = "linux")]
            Self::LockedEagerly => "全部映射已预先锁定（内核无缺页锁定）",
        }
    }
}

/// 本平台不提供该能力；只登记事实，不冒称存在等价替代。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unsupported {
    /// macOS 没有 prctl 的可转储开关，也没有 `/proc` 这条同 UID 读取路径；
    /// 同名保护不存在，本进程不宣称已获得与 Linux 等价的结果。
    #[cfg(not(target_os = "linux"))]
    NoDumpableSwitch,
    /// macOS 没有可用的 `mlockall`；按页 `mlock` 需要 unsafe，工程禁用 unsafe。
    #[cfg(not(target_os = "linux"))]
    NoAddressSpaceLock,
    /// 系统环境块未被清除，但本次实测的可检视属性已挡住同 UID 的外部读取。
    ///
    /// 内核在初始栈上保留的那份环境字符串无法在禁用 unsafe 的前提下改写：
    /// `std::env::remove_var` 在 edition 2024 是 unsafe 且工程禁用 unsafe，
    /// 即便允许，它也只摘掉 libc 的指针表，`/proc/<pid>/environ` 仍读出原值。
    /// 真正挡住外部读取的是 [`Measure::ProcessInspection`]，不是清除本身。
    EnvironmentBlockShieldedByInspection,
    /// 系统环境块未被清除，且本次实测没有任何措施挡住同 UID 的外部读取。
    ///
    /// macOS 属于这一档：`KERN_PROCARGS2` 对同 UID 进程开放，本进程无法关闭。
    EnvironmentBlockReadableBySameUser,
}

impl Unsupported {
    fn label(self) -> &'static str {
        match self {
            #[cfg(not(target_os = "linux"))]
            Self::NoDumpableSwitch => "本平台没有可转储属性开关",
            #[cfg(not(target_os = "linux"))]
            Self::NoAddressSpaceLock => "本平台没有可用的地址空间锁定",
            // 标签里不得出现「；」：那是摘要里分隔措施用的，混进来会读不出项数。
            Self::EnvironmentBlockShieldedByInspection => {
                "系统环境副本未清除（自有副本仍零化，外部读取已被可检视属性挡住）"
            }
            Self::EnvironmentBlockReadableBySameUser => {
                "系统环境副本未清除，且同用户进程仍可读出（自有副本仍零化）"
            }
        }
    }
}

/// 单项措施的固定结果；没有第三种取值，缺失能力不会伪装成成功。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// 已生效并回读确认。
    Applied(Applied),
    /// 本平台不提供。
    Unsupported(Unsupported),
}

impl fmt::Display for Outcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Applied(applied) => write!(formatter, "已生效：{}", applied.label()),
            Self::Unsupported(reason) => write!(formatter, "不支持：{}", reason.label()),
        }
    }
}

/// 本平台声明支持、却没能真正生效的原因；出现即整体失败，不读取秘密。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cause {
    /// 系统调用被拒绝。
    Rejected,
    /// 调用返回成功，但回读结果与预期不符。
    Unconfirmed,
    /// 可锁定内存上限不足以覆盖本进程；提高 `RLIMIT_MEMLOCK` 后重试。
    #[cfg(target_os = "linux")]
    MemoryLockLimit,
}

impl Cause {
    fn label(self) -> &'static str {
        match self {
            Self::Rejected => "系统调用被拒绝",
            Self::Unconfirmed => "回读结果与预期不符",
            #[cfg(target_os = "linux")]
            Self::MemoryLockLimit => "可锁定内存上限不足，请提高 RLIMIT_MEMLOCK 后重试",
        }
    }
}

/// 加固失败；调用方必须在未读取任何秘密的情况下终止。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    measure: Measure,
    cause: Cause,
}

impl Refusal {
    pub(crate) fn new(measure: Measure, cause: Cause) -> Self {
        Self { measure, cause }
    }

    #[cfg(test)]
    pub(crate) fn cause(&self) -> Cause {
        self.cause
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "无法在读取秘密前收紧当前进程的{}：{}；未读取秘密",
            self.measure.label(),
            self.cause.label()
        )
    }
}

impl std::error::Error for Refusal {}

/// 一次加固的完整结果；四项措施各有且只有一个固定分类，不存在缺项。
///
/// 只经 [`Display`](fmt::Display) 给出固定的非秘密摘要：调用方把它原样记录下来，
/// 操作者才看得见本平台哪几项不提供。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Residency([Outcome; Measure::COUNT]);

impl Residency {
    pub(crate) fn new(outcomes: [Outcome; Measure::COUNT]) -> Self {
        Self(outcomes)
    }

    /// 查询单项措施的分类；措施集合封闭，永远查得到，不存在「未知」。
    #[cfg(test)]
    pub(crate) fn outcome(&self, measure: Measure) -> Outcome {
        self.0[measure.index()]
    }
}

impl fmt::Display for Residency {
    /// 固定的非秘密摘要：只含措施名与分类，永不含种子、验证码或环境值。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, (measure, outcome)) in Measure::ORDER.into_iter().zip(self.0).enumerate() {
            if index > 0 {
                formatter.write_str("；")?;
            }
            write!(formatter, "{measure}={outcome}")?;
        }
        Ok(())
    }
}
