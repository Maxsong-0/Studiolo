//! 完整校验的不可变副本；只持有 Broker 只读能力，不直接访问宿主文件系统。

mod protocol;
mod verify;

use std::path::Path;

use studiolo_core::VaultPath;
use studiolo_vault_broker::ReadOnlyVault;

use crate::Snapshot;
use protocol::State;
pub use protocol::{NOTES_TOTAL_LIMIT, Receipt};

/// 固定原因，不含源文件内容、宿主路径或底层错误。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReplicaError {
    /// 控制文件、时间、根或清单不合法。
    #[error("replica_invalid: {0}")]
    Invalid(&'static str),
    /// 已发布内容与清单不一致。
    #[error("replica_drift: {0}")]
    Drift(&'static str),
    /// 发布指针在验证期间改变；有界重试仍未获得一致代次。
    #[error("replica_superseded: 发布持续变化，请重试")]
    Superseded,
}

/// 与已验证内存快照绑定的新鲜度；重扫时间不覆盖来源时间。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Freshness {
    /// 本批来源回执。
    pub receipt: Receipt,
    /// 最近一次同步尝试的 Unix 毫秒时间。
    pub last_attempt_at_ms: i64,
    /// 最近一次尝试失败原因；成功时为空。
    pub last_error: Option<String>,
    /// 超过此秒数显示 stale。
    pub max_age_secs: u64,
}

impl Freshness {
    /// 动态计算 age；状态展示与内容同一代次。
    ///
    /// 第一行是固定的机器行（上线探针按整行匹配，任何字符都不能变）；`stale` 或 `sync_failed`
    /// 时在它下一行追加一句给人看的提示，`ready` 时只有机器行。
    pub fn describe(&self, now_ms: i64) -> Result<String, ReplicaError> {
        self.state().validate(now_ms)?;
        let observed = self
            .receipt
            .source_observed_at_ms
            .min(self.receipt.received_at_ms);
        let age_ms = now_ms.saturating_sub(observed).max(0);
        let stale =
            age_ms > i64::try_from(self.max_age_secs.saturating_mul(1000)).unwrap_or(i64::MAX);
        let status = if self.last_error.is_some() {
            "sync_failed"
        } else if stale {
            "stale"
        } else {
            "ready"
        };
        let machine = format!(
            "副本状态 {status}；stale={stale}；generation={}；source_digest={}；source_observed_at_ms={}；received_at_ms={}；age_secs={}；max_age_secs={}{}",
            self.receipt.generation,
            self.receipt.source_digest,
            self.receipt.source_observed_at_ms,
            self.receipt.received_at_ms,
            age_ms / 1000,
            self.max_age_secs,
            self.last_error
                .as_ref()
                .map_or(String::new(), |error| format!(
                    "；last_error={error}（读取最后验证副本）"
                ))
        );
        Ok(match notice(status, observed, age_ms) {
            Some(hint) => format!("{machine}\n{hint}"),
            None => machine,
        })
    }

    fn state(&self) -> State {
        State {
            schema_version: 1,
            status: if self.last_error.is_some() {
                "failed"
            } else {
                "ready"
            }
            .into(),
            last_attempt_at_ms: self.last_attempt_at_ms,
            last_error: self.last_error.clone(),
            receipt: Some(self.receipt.clone()),
        }
    }
}

/// Markdown 合计用到上限的这个百分比起，路牌上提醒（T-215，vault BUG-039：越线那天 Mac 同步整批失败、
/// 副本停在越线前，之前没有任何提前量，等发现时已经停了一个月）。按每月约 5 MiB 的增长留出两个月左右。
pub const NOTES_TOTAL_WARN_PERCENT: u64 = 80;

/// 路牌上给人看的容量提醒：Markdown 合计到了上限的 [`NOTES_TOTAL_WARN_PERCENT`] 才有。
///
/// 只放在路牌上（每个 AI 会话开头都会读到，会转告用户），不放进逐次的内容响应。
#[must_use]
pub fn capacity_notice(notes_bytes: u64) -> Option<String> {
    let percent = notes_bytes.saturating_mul(100) / NOTES_TOTAL_LIMIT;
    (percent >= NOTES_TOTAL_WARN_PERCENT).then(|| {
        let mib = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
        format!(
            "⚠ 副本 Markdown 合计 {:.1} MiB，已到上限 {:.0} MiB 的 {percent}%：涨过上限后 Mac 同步会整批失败、\
             这里的资料停在越线前（vault BUG-039）。请提醒用户趁早调高上限（两端常量，见 docs/m3/replica-freshness.md）\
             或精简白名单。",
            mib(notes_bytes),
            mib(NOTES_TOTAL_LIMIT),
        )
    })
}

/// 机器行下面那句给人看的提示；`ready` 没有。
///
/// AI 从 `age_secs=1836305` 很难看出资料已停了 21 天，所以把停在哪一刻、停了多久写成人话。
/// 毫秒值越界、时间写不出来时宁可不加提示，也不能让机器行跟着失败。
fn notice(status: &str, observed_ms: i64, age_ms: i64) -> Option<String> {
    if status != "stale" && status != "sync_failed" {
        return None;
    }
    let stopped = jiff::Timestamp::from_millisecond(observed_ms)
        .ok()?
        .strftime("%Y-%m-%d %H:%M")
        .to_string();
    let ago = elapsed(age_ms);
    Some(if status == "stale" {
        format!(
            "⚠ 资料停在 {stopped} UTC（{ago}前），之后的改动这里看不到；涉及进度的结论先向用户核实。"
        )
    } else {
        format!(
            "⚠ 最近一次同步失败，下面是最后一次校验通过的资料：停在 {stopped} UTC（{ago}前）；涉及进度的结论先向用户核实。"
        )
    })
}

/// 「多久」：不足 1 小时按分钟（最少 1），不足 48 小时按小时，其余按天，都向下取整。
fn elapsed(age_ms: i64) -> String {
    const MINUTE_MS: i64 = 60_000;
    const HOUR_MS: i64 = 60 * MINUTE_MS;
    const DAY_MS: i64 = 24 * HOUR_MS;
    let age_ms = age_ms.max(0);
    if age_ms < HOUR_MS {
        format!("{} 分钟", (age_ms / MINUTE_MS).max(1))
    } else if age_ms < 48 * HOUR_MS {
        format!("{} 小时", age_ms / HOUR_MS)
    } else {
        format!("{} 天", age_ms / DAY_MS)
    }
}

/// 一个完整校验的代次，不含任何仍指向可变文件的句柄。
pub struct VerifiedReplica {
    /// 供给用的完整内存快照。
    pub snapshot: Snapshot,
    /// 与快照绑定的来源状态。
    pub freshness: Freshness,
}

/// 在启动时固定的私有副本根；白名单必须与每次发布清单完全一致。
pub struct ReplicaRoot {
    root: ReadOnlyVault,
    allow: Vec<VaultPath>,
    max_age_secs: u64,
}

impl ReplicaRoot {
    /// 打开现有私有根；任何目录都不会被创建。
    pub fn open(root: &Path, allow: &[String], max_age_secs: u64) -> Result<Self, ReplicaError> {
        if !(60..=86_400).contains(&max_age_secs) {
            return Err(ReplicaError::Invalid("max_age_range"));
        }
        Ok(Self {
            root: ReadOnlyVault::open_private(root)
                .map_err(|_| ReplicaError::Invalid("unsafe_root"))?,
            allow: protocol::allowlist(allow)?,
            max_age_secs,
        })
    }

    /// 有界校验全部清单、文件字节与可见目录；前后控制文件必须逐字节一致。
    /// 发布并发改变时最多重试三次；旧回执不得回退或改绑另一内容。
    pub fn verify(
        &self,
        now_ms: i64,
        previous: Option<&Freshness>,
    ) -> Result<VerifiedReplica, ReplicaError> {
        for _ in 0..3 {
            let before = self.control()?;
            let state: State =
                serde_json::from_slice(&before).map_err(|_| ReplicaError::Invalid("state_json"))?;
            let receipt = state.validate(now_ms)?;
            if let Some(previous) = previous {
                let old = &previous.receipt;
                if receipt.source_observed_at_ms < old.source_observed_at_ms
                    || receipt.received_at_ms < old.received_at_ms
                    || state.last_attempt_at_ms < previous.last_attempt_at_ms
                    || (receipt.generation == old.generation
                        && receipt.source_digest != old.source_digest)
                {
                    return Err(ReplicaError::Invalid("receipt_regression"));
                }
            }
            let verified = verify::generation(&self.root, &self.allow, receipt);
            let after = self.control()?;
            if before != after {
                continue;
            }
            return Ok(VerifiedReplica {
                snapshot: verified?,
                freshness: Freshness {
                    receipt: receipt.clone(),
                    last_attempt_at_ms: state.last_attempt_at_ms,
                    last_error: state.last_error,
                    max_age_secs: self.max_age_secs,
                },
            });
        }
        Err(ReplicaError::Superseded)
    }

    fn control(&self) -> Result<Vec<u8>, ReplicaError> {
        let path = VaultPath::parse("control/state.json").expect("fixed path");
        self.root
            .read_file(&path, 4096)
            .map(|note| note.content)
            .map_err(|_| ReplicaError::Invalid("unreadable_state"))
    }
}

#[cfg(test)]
mod tests {
    use super::{NOTES_TOTAL_LIMIT, NOTES_TOTAL_WARN_PERCENT, capacity_notice, elapsed, notice};

    const SECOND_MS: i64 = 1000;
    const MINUTE_MS: i64 = 60 * SECOND_MS;
    const HOUR_MS: i64 = 60 * MINUTE_MS;
    // 2026-09-21 13:31:27.879 UTC；秒与毫秒不进提示，只到分钟。
    const OBSERVED_MS: i64 = 1_789_997_487_879;

    #[test]
    fn elapsed_rounds_down_at_each_unit_boundary() {
        assert_eq!(elapsed(0), "1 分钟");
        assert_eq!(elapsed(59 * SECOND_MS), "1 分钟");
        assert_eq!(elapsed(59 * MINUTE_MS + 59 * SECOND_MS), "59 分钟");
        assert_eq!(elapsed(HOUR_MS), "1 小时");
        assert_eq!(elapsed(47 * HOUR_MS + 59 * MINUTE_MS), "47 小时");
        assert_eq!(elapsed(48 * HOUR_MS), "2 天");
        assert_eq!(elapsed(21 * 24 * HOUR_MS + 5 * HOUR_MS), "21 天");
        assert_eq!(elapsed(-5), "1 分钟");
    }

    #[test]
    fn notice_is_absent_when_ready_and_spells_out_time_when_stale_or_failed() {
        assert_eq!(notice("ready", OBSERVED_MS, 48 * HOUR_MS), None);
        assert_eq!(
            notice("stale", OBSERVED_MS, 59 * MINUTE_MS + 59 * SECOND_MS).as_deref(),
            Some(
                "⚠ 资料停在 2026-09-21 13:31 UTC（59 分钟前），之后的改动这里看不到；涉及进度的结论先向用户核实。"
            )
        );
        assert_eq!(
            notice("sync_failed", OBSERVED_MS, HOUR_MS).as_deref(),
            Some(
                "⚠ 最近一次同步失败，下面是最后一次校验通过的资料：停在 2026-09-21 13:31 UTC（1 小时前）；涉及进度的结论先向用户核实。"
            )
        );
    }

    #[test]
    fn capacity_notice_starts_at_the_warning_percent_of_the_limit() {
        let at = |percent: u64| (NOTES_TOTAL_LIMIT * percent).div_ceil(100);
        assert_eq!(capacity_notice(0), None);
        assert_eq!(capacity_notice(at(NOTES_TOTAL_WARN_PERCENT) - 1), None);
        let warned = capacity_notice(at(NOTES_TOTAL_WARN_PERCENT)).expect("到线就提醒");
        assert!(
            warned.contains(&format!("的 {NOTES_TOTAL_WARN_PERCENT}%")),
            "{warned}"
        );
        let full = capacity_notice(NOTES_TOTAL_LIMIT).expect("满额也提醒");
        assert!(full.starts_with("⚠ 副本 Markdown 合计 "), "{full}");
        assert!(
            full.contains("的 100%") && full.contains("BUG-039"),
            "{full}"
        );
    }

    #[test]
    fn unformattable_time_drops_the_notice_instead_of_failing() {
        assert_eq!(notice("stale", i64::MAX, HOUR_MS), None);
        assert_eq!(notice("sync_failed", i64::MIN, HOUR_MS), None);
    }
}
