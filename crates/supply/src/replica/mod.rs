//! 完整校验的不可变副本；只持有 Broker 只读能力，不直接访问宿主文件系统。

mod protocol;
mod verify;

use std::path::Path;

use studiolo_core::VaultPath;
use studiolo_vault_broker::ReadOnlyVault;

use crate::Snapshot;
use protocol::{ENTRY_LIMIT, MANIFEST_LIMIT, State, TOTAL_LIMIT};
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

/// 一个代次用了副本协议各条上限多少。校验时按清单算，与越线判定是同一组数。
///
/// 四条上限都会被库的自然增长顶穿，越线那天 Mac 同步整批失败、副本停在越线前（vault BUG-039 越的是
/// Markdown 合计；T-215 复查指出另外三条同样没有提前量），所以四条一起看。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// 清单原始字节。
    pub manifest_bytes: u64,
    /// 清单条目数。只含可见项：Mac 端捕获时连隐藏项一起数（`replica_capture.py` 的 `inspected`），实际更多。
    pub entries: usize,
    /// 全部文件字节（Markdown 与附件）。
    pub total_bytes: u64,
    /// Markdown 合计字节。
    pub notes_bytes: u64,
}

/// 与已验证内存快照绑定的来源状态；重扫时间不覆盖来源时间。
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
    /// 本代次用了各条协议上限多少；只供路牌的容量提醒（[`capacity_notice_for`]）。
    pub usage: Usage,
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

/// 任一条协议上限用到这个百分比起，路牌上提醒（T-215，vault BUG-039：越线那天 Mac 同步整批失败、
/// 副本停在越线前，之前没有任何提前量，等发现时已经停了一个月）。按每月约 5 MiB 的增长留出两个月左右。
pub const CAPACITY_WARN_PERCENT: u64 = 80;

/// 路牌上给人看的容量提醒：这一代用到某条协议上限的 [`CAPACITY_WARN_PERCENT`] 才有，
/// 而且只给读得到副本里**每一个条目**的接入方（`readable` 是这个接入方的读授权）。
///
/// 用量是全副本的数：只读得到一部分的接入方拿到它，减去自己读得到的部分，就能推出读不到的项目
/// 和名单外规则有多大、有多少附件。所以读不全就不给，宁可少提醒，也不越过供给视图（T-215 外部审查发现）。
/// 只放在路牌上（每个 AI 会话开头都会读到，会转告用户），不放进逐次的内容响应。
#[must_use]
pub fn capacity_notice_for(
    snapshot: &Snapshot,
    usage: &Usage,
    mut readable: impl FnMut(&VaultPath) -> bool,
) -> Option<String> {
    if !snapshot.entries.iter().all(|entry| readable(&entry.path)) {
        return None;
    }
    capacity_notice(usage)
}

/// 提醒的文字；不看授权，只供 [`capacity_notice_for`] 调用。四条上限里到线的逐条列出。
fn capacity_notice(usage: &Usage) -> Option<String> {
    let gauges = [
        Gauge::bytes("Markdown 合计", usage.notes_bytes, NOTES_TOTAL_LIMIT),
        Gauge::bytes("文件总量", usage.total_bytes, TOTAL_LIMIT),
        // 只数可见项，Mac 端连隐藏项一起数（`Usage::entries`），所以这条按偏少的数提醒。
        Gauge::count("可见条目", usage.entries as u64, ENTRY_LIMIT as u64),
        Gauge::bytes("清单", usage.manifest_bytes, MANIFEST_LIMIT),
    ];
    let reached = gauges.iter().filter_map(Gauge::reached).collect::<Vec<_>>();
    (!reached.is_empty()).then(|| {
        format!(
            "⚠ 副本快到协议上限：{}。涨过任一条上限后 Mac 同步会整批失败、这里的资料停在越线前（vault BUG-039）。\
             请提醒用户趁早调高上限（两端常量，见 docs/m3/replica-freshness.md）或精简白名单。",
            reached.join("；")
        )
    })
}

/// 一条协议上限的用量。
struct Gauge {
    name: &'static str,
    used: u64,
    limit: u64,
    /// 按字节（显示成 MiB）还是按个数。
    in_bytes: bool,
}

impl Gauge {
    fn bytes(name: &'static str, used: u64, limit: u64) -> Self {
        Self {
            name,
            used,
            limit,
            in_bytes: true,
        }
    }

    fn count(name: &'static str, used: u64, limit: u64) -> Self {
        Self {
            name,
            used,
            limit,
            in_bytes: false,
        }
    }

    /// 到了提醒线就给出这一条：用了多少、上限多少、百分之几（向下取整）。
    fn reached(&self) -> Option<String> {
        let percent = self.used.saturating_mul(100) / self.limit;
        (percent >= CAPACITY_WARN_PERCENT).then(|| {
            format!(
                "{} {}，上限 {} 的 {percent}%",
                self.name,
                self.show(self.used),
                self.show(self.limit)
            )
        })
    }

    fn show(&self, value: u64) -> String {
        if self.in_bytes {
            format!("{:.1} MiB", value as f64 / (1024.0 * 1024.0))
        } else {
            value.to_string()
        }
    }
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
            let (snapshot, usage) = verified?;
            return Ok(VerifiedReplica {
                snapshot,
                freshness: Freshness {
                    receipt: receipt.clone(),
                    last_attempt_at_ms: state.last_attempt_at_ms,
                    last_error: state.last_error,
                    max_age_secs: self.max_age_secs,
                    usage,
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
    use studiolo_core::VaultPath;

    use super::{
        CAPACITY_WARN_PERCENT, ENTRY_LIMIT, MANIFEST_LIMIT, NOTES_TOTAL_LIMIT, TOTAL_LIMIT, Usage,
        capacity_notice, capacity_notice_for, elapsed, notice,
    };
    use crate::{Entry, EntryKind, Snapshot};

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

    /// 改 [`Usage`] 里的一项。
    type SetUsage = fn(&mut Usage, u64);

    /// 用量刚好到某个百分比（向上取整）。
    fn at(limit: u64, percent: u64) -> u64 {
        (limit * percent).div_ceil(100)
    }

    #[test]
    fn capacity_notice_starts_at_the_warning_percent_of_each_limit() {
        assert_eq!(capacity_notice(&Usage::default()), None);
        // 四条上限各自到线就提醒，差一点不提醒；提醒里点名是哪一条。
        let cases: [(&str, SetUsage, u64); 4] = [
            ("Markdown 合计", |u, v| u.notes_bytes = v, NOTES_TOTAL_LIMIT),
            ("文件总量", |u, v| u.total_bytes = v, TOTAL_LIMIT),
            (
                "可见条目",
                |u, v| u.entries = usize::try_from(v).unwrap(),
                ENTRY_LIMIT as u64,
            ),
            ("清单", |u, v| u.manifest_bytes = v, MANIFEST_LIMIT),
        ];
        for (name, set, limit) in cases {
            let mut usage = Usage::default();
            set(&mut usage, at(limit, CAPACITY_WARN_PERCENT) - 1);
            assert_eq!(capacity_notice(&usage), None, "{name} 差一点到线");
            set(&mut usage, at(limit, CAPACITY_WARN_PERCENT));
            let warned = capacity_notice(&usage).unwrap_or_else(|| panic!("{name} 到线就提醒"));
            assert!(
                warned.starts_with("⚠ 副本快到协议上限：")
                    && warned.contains(&format!("{name} "))
                    && warned.contains(&format!("的 {CAPACITY_WARN_PERCENT}%"))
                    && warned.contains("BUG-039"),
                "{warned}"
            );
            set(&mut usage, limit);
            let full = capacity_notice(&usage).expect("满额也提醒");
            assert!(full.contains("的 100%"), "{full}");
        }
        // 同时到线的逐条列出。
        let both = capacity_notice(&Usage {
            notes_bytes: NOTES_TOTAL_LIMIT,
            total_bytes: TOTAL_LIMIT,
            ..Usage::default()
        })
        .expect("两条都到线");
        assert!(
            both.contains("Markdown 合计 40.0 MiB，上限 40.0 MiB 的 100%；文件总量 "),
            "{both}"
        );
    }

    #[test]
    fn capacity_notice_is_withheld_from_a_reader_who_cannot_read_every_entry() {
        let entry = |path: &str, kind: EntryKind| Entry {
            path: VaultPath::parse(path).unwrap(),
            kind,
            size_bytes: (kind != EntryKind::Directory).then_some(1),
        };
        let snapshot = Snapshot {
            entries: vec![
                entry("_规则/通用工作规则.md", EntryKind::Markdown),
                entry("项目/Public/a.md", EntryKind::Markdown),
                entry("项目/Secret", EntryKind::Directory),
                entry("项目/Secret/b.md", EntryKind::Markdown),
                entry("项目/Secret/c.pdf", EntryKind::Attachment),
            ],
            ..Snapshot::default()
        };
        let full = Usage {
            notes_bytes: NOTES_TOTAL_LIMIT,
            ..Usage::default()
        };
        let everything =
            capacity_notice_for(&snapshot, &full, |_| true).expect("读全的接入方看得到提醒");
        assert!(
            everything.starts_with("⚠ 副本快到协议上限："),
            "{everything}"
        );
        assert_eq!(
            capacity_notice_for(&snapshot, &full, |path| !path
                .as_str()
                .starts_with("项目/Secret")),
            None,
            "只读得到一部分的接入方不能拿到全副本的用量"
        );
        assert_eq!(
            capacity_notice_for(&snapshot, &full, |path| path.as_str()
                != "_规则/通用工作规则.md"),
            None,
            "规则名单外的一篇也算读不全"
        );
        assert_eq!(
            capacity_notice_for(&snapshot, &full, |path| path.as_str()
                != "项目/Secret/c.pdf"),
            None,
            "读不到的附件也算读不全：文件总量和条目数会泄露它"
        );
        assert_eq!(
            capacity_notice_for(&snapshot, &Usage::default(), |_| true),
            None,
            "没到线不提醒"
        );
    }

    #[test]
    fn unformattable_time_drops_the_notice_instead_of_failing() {
        assert_eq!(notice("stale", i64::MAX, HOUR_MS), None);
        assert_eq!(notice("sync_failed", i64::MIN, HOUR_MS), None);
    }
}
