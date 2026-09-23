//! cgroup v2 事实的纯解析与比对：只认「同一子组实测为空」，不接受任何替代证据。

use super::ContainmentGap;
use crate::owned_run::RunBudget;

/// `cgroup.events` 的解析结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CgroupEvents {
    /// 本组及全部后代是否仍有存活进程；`false` 才是空组。
    pub populated: bool,
    /// 是否处于 frozen 状态。
    pub frozen: bool,
}

/// 严格解析 `cgroup.events`：缺 `populated`、重复键或未知键一律拒绝。
///
/// # 错误
/// 格式不符返回 `ProtocolViolation`。
pub fn parse_events(text: &str) -> Result<CgroupEvents, ContainmentGap> {
    let mut populated = None;
    let mut frozen = None;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let (key, value) = line
            .split_once(' ')
            .ok_or(ContainmentGap::ProtocolViolation)?;
        let flag = match value {
            "0" => false,
            "1" => true,
            _ => return Err(ContainmentGap::ProtocolViolation),
        };
        let slot = match key {
            "populated" => &mut populated,
            "frozen" => &mut frozen,
            _ => return Err(ContainmentGap::ProtocolViolation),
        };
        if slot.replace(flag).is_some() {
            return Err(ContainmentGap::ProtocolViolation);
        }
    }
    Ok(CgroupEvents {
        populated: populated.ok_or(ContainmentGap::ProtocolViolation)?,
        frozen: frozen.unwrap_or(false),
    })
}

/// 实测的受控子组限额；`None` 表示内核报告为 `max`（无限）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CgroupLimits {
    /// `memory.max`。
    pub memory_max: Option<u64>,
    /// `memory.swap.max`。
    pub memory_swap_max: Option<u64>,
    /// `pids.max`。
    pub pids_max: Option<u64>,
    /// `cpu.max` 的 `(quota, period)`。
    pub cpu_max: Option<(u64, u64)>,
}

/// 解析 `memory.max`/`memory.swap.max`/`pids.max` 这类「数值或 max」的单值文件。
///
/// # 错误
/// 非十进制数值且非 `max` 时返回 `ProtocolViolation`。
pub fn parse_scalar(text: &str) -> Result<Option<u64>, ContainmentGap> {
    let value = text.trim();
    if value == "max" {
        return Ok(None);
    }
    value
        .parse::<u64>()
        .map(Some)
        .map_err(|_| ContainmentGap::ProtocolViolation)
}

/// 解析 `cpu.max` 的 `"<quota|max> <period>"`。
///
/// # 错误
/// 字段数不对、周期为零或数值非法时返回 `ProtocolViolation`。
pub fn parse_cpu_max(text: &str) -> Result<Option<(u64, u64)>, ContainmentGap> {
    let mut parts = text.split_whitespace();
    let quota = parts.next().ok_or(ContainmentGap::ProtocolViolation)?;
    let period = parts
        .next()
        .ok_or(ContainmentGap::ProtocolViolation)?
        .parse::<u64>()
        .map_err(|_| ContainmentGap::ProtocolViolation)?;
    if parts.next().is_some() || period == 0 {
        return Err(ContainmentGap::ProtocolViolation);
    }
    if quota == "max" {
        return Ok(None);
    }
    let quota = quota
        .parse::<u64>()
        .map_err(|_| ContainmentGap::ProtocolViolation)?;
    Ok(Some((quota, period)))
}

/// 实际限额必须逐项满足预算；「已请求」不算，任何一项是无限就拒绝启动。
///
/// # 错误
/// 任一项缺失或宽于预算返回 `BudgetUnsupported`。
pub fn satisfies(limits: &CgroupLimits, budget: &RunBudget) -> Result<(), ContainmentGap> {
    let memory = limits
        .memory_max
        .is_some_and(|value| value > 0 && value <= budget.memory_bytes);
    // 不给任何 swap 预算：否则内存上限可以被换页绕开。
    let swap = limits.memory_swap_max == Some(0);
    let pids = limits
        .pids_max
        .is_some_and(|value| value > 0 && value <= u64::from(budget.pids_max));
    let cpu = limits.cpu_max.is_some_and(|(quota, period)| {
        quota
            .checked_mul(100)
            .is_some_and(|scaled| scaled <= u64::from(budget.cpu_max_percent) * period)
    });
    if memory && swap && pids && cpu {
        Ok(())
    } else {
        Err(ContainmentGap::BudgetUnsupported)
    }
}

/// 受控子组路径必须落在批准前缀之内，且不得用 `.`/`..` 或空段绕出。
#[must_use]
pub fn within_prefix(path: &str, prefix: &str) -> bool {
    crate::owned_run::absolute_normal(std::path::Path::new(path))
        && path.len() > prefix.len()
        && path.starts_with(prefix)
        && path.as_bytes().get(prefix.len()) == Some(&b'/')
}
