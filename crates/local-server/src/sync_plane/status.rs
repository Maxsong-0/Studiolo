//! 同步状态回执 `<plane_root>/sync-status.json` 的纯编解码与重启起点（T-224，BUG-046 防回归）；没有 IO。
//!
//! 人侧每轮写一次，F4 的 169 探针（`deploy/alerts/probe.py`）读它判「同步停摆」（A07）与「删除被挡」（A08）。
//! 人侧只写事实，6 小时、15 分钟两条线在探针里；字段与读法见 `docs/contracts/sync-status-receipt.md`。
//! 它也是「两边最后一次对上」跨重启的唯一落点：不进 `plane-state.json`——那份拒绝未知字段，
//! 改了它新发行就退不回上一版。读写只经 `recovery::PlaneStateStore` 的两个方法。

use super::PlaneError;
use super::driver::RoundOutcome;

/// 回执版本，固定为 1。
pub const SCHEMA: u8 = 1;
/// 回执字节上限；探针按同一上限读。
pub const MAX_BYTES: usize = 4096;
/// 回执的字段个数：闭合四字段，一个都不能少。
const FIELDS: usize = 4;

/// 同步状态回执。时刻都是 Unix 秒；`None` 写成 JSON `null`，字段本身总在。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusReceipt {
    /// 固定为 [`SCHEMA`]。
    pub schema: u8,
    /// 这份回执写出的时刻；探针据此判断「删除被挡」还算不算数。
    pub written_at: i64,
    /// 两边最后一次对上的那一轮开始的时刻；`None` 即说不清（上一份回执坏了），探针按停摆报。
    pub last_aligned_at: Option<i64>,
    /// 最近一次走到删除守卫那一轮被挡的主库删除数（BUG-046）；`None` 即还没有一轮证明过。
    pub deletes_blocked: Option<u64>,
}

/// 开平面时读到的上一份回执。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Previous {
    /// 没有回执：首次运行带本功能的发行、新建的平面或恢复之后。
    Absent,
    /// 读到一份合法回执。
    Valid(StatusReceipt),
    /// 有回执但读不出合法内容（权限、属主、大小、字段任一不对）。
    Corrupt,
}

/// 重启后接着用的两项事实。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resumed {
    /// 「最后对上」的起点。
    pub last_aligned_at: Option<i64>,
    /// 上一个进程最后报的删除被挡数：本进程的删除守卫跑过之前沿用它，只进回执、不进健康第六节。
    pub deletes_blocked: Option<u64>,
}

/// 一轮是否证明两边对上：`Idle`（规划结果全是共同观察）或 `Settled`（一批闭合）。
/// 别的结论（卡在终态未知、冻结、删除被挡、退避、强制恢复、任何错误）都不算。
#[must_use]
pub fn aligned(outcome: &Result<RoundOutcome, PlaneError>) -> bool {
    matches!(
        outcome,
        Ok(RoundOutcome::Idle | RoundOutcome::Settled { .. })
    )
}

/// 按上一份回执给出重启起点。没有回执从 `now` 起算（首装不误报）；回执坏了、
/// 或记着的对上时刻晚于 `now`（时钟回拨，说不清过了多久）就记空，让探针按停摆报。
#[must_use]
pub fn resume(previous: &Previous, now: i64) -> Resumed {
    match previous {
        Previous::Absent => Resumed {
            last_aligned_at: Some(now),
            deletes_blocked: None,
        },
        Previous::Valid(receipt) => Resumed {
            last_aligned_at: receipt.last_aligned_at.filter(|at| *at <= now),
            deletes_blocked: receipt.deletes_blocked,
        },
        Previous::Corrupt => Resumed {
            last_aligned_at: None,
            deletes_blocked: None,
        },
    }
}

/// 编码成写盘的字节（紧凑 JSON，四个字段都在）。
///
/// # Errors
/// 序列化失败或超过 [`MAX_BYTES`] 返回 `Storage`。
pub fn encode(receipt: &StatusReceipt) -> Result<Vec<u8>, PlaneError> {
    let bytes = serde_json::to_vec(receipt).map_err(|_| PlaneError::Storage)?;
    if bytes.len() > MAX_BYTES {
        return Err(PlaneError::Storage);
    }
    Ok(bytes)
}

/// 解码并核对一份回执：超限、不是 JSON 对象、缺字段、多字段、重复键、类型或版本不对、
/// 时刻为负、对上时刻晚于写出时刻，一律 `None`。
#[must_use]
pub fn decode(bytes: &[u8]) -> Option<StatusReceipt> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    // 结构体解码挡住未知字段、重复键和类型；缺了可空字段 serde 会补 None，所以另数一遍键。
    let keys = serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(bytes).ok()?;
    let receipt = serde_json::from_slice::<StatusReceipt>(bytes).ok()?;
    let consistent = keys.len() == FIELDS
        && receipt.schema == SCHEMA
        && receipt.written_at >= 0
        && receipt
            .last_aligned_at
            .is_none_or(|at| (0..=receipt.written_at).contains(&at));
    consistent.then_some(receipt)
}

#[cfg(test)]
#[path = "../../tests/sync_plane_private/status.rs"]
mod tests;
