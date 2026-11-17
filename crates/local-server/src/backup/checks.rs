//! 仓库抽查以账本为准：从抽查事件里算出下一个检查序号、挂着的原意图、各组最近的结果，
//! 以及该不该查、查哪组。只有纯逻辑，按账本顺序一条条喂；读写账本在 `producer.rs`。
//! 没有单独的状态文件：进程重启后把账本里的抽查事件重读一遍就得到同样的结论。
//!
//! 节奏全部从账本里的时刻算：
//! - 轮转：距上一次轮转的原意图满 24 小时，查上次轮转那组的下一组（第一次查第 1 组）。
//!   结果好坏都往下轮：一组一直不干净也不会卡住整圈，它另走重查。
//! - 重查：一组最近一次结果不干净，第一次不干净 1 小时后重查，连续两次以上不干净后每 24 小时
//!   重查一次，直到干净；几组都要重查时先查结果最早的那组；两次重查的原意图之间至少隔 1 小时。
//! - 重查和轮转同时到期先重查。挂着的原意图没挂满 `ABANDON_AFTER_MS`（抽查窗口加 10 分钟余量）
//!   算还在查，挂满了就由生产者补「没做完」。worker 只在空闲、手里没有没交出去的结果时才问，
//!   所以它来问时挂着的原意图只可能是上一个进程中途没了留下的。
//! - 换了仓库（worker 上报的仓库摘要变了），旧仓库各组的结果不再算。
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use studiolo_audit_ledger::AuditRecord;
use studiolo_backup::worker::{
    recovery_point::BackupTargetKind,
    repository_check::{
        CHECK_WINDOW, CheckBucket, CheckEvent, CheckKey, CheckOutcome, CheckPurpose, CheckRecorder,
        CheckResult, RepositoryDigest, read_check_event,
    },
};

const HOUR_MS: i64 = 3_600_000;
/// 轮转间隔：30 组一圈，约 30 天读完整个仓库。
pub(super) const ROTATION_INTERVAL_MS: i64 = 24 * HOUR_MS;
/// 第一次不干净之后多久重查。
pub(super) const FIRST_RETRY_AFTER_MS: i64 = HOUR_MS;
/// 连续两次以上不干净之后多久重查一次。
pub(super) const REPEAT_RETRY_AFTER_MS: i64 = 24 * HOUR_MS;
/// 两次重查的原意图之间至少隔多久。
pub(super) const RETRY_SPACING_MS: i64 = HOUR_MS;
/// 原意图挂了多久算 worker 中途没了：worker 一次最多查 `CHECK_WINDOW`（20 分钟），再留 10 分钟余量，
/// 共 30 分钟。时限比窗口短的话，还在查的那组会被补成「中途没了」，worker 交来的结果被拒。
pub(super) const ABANDON_AFTER_MS: i64 = CHECK_WINDOW.as_millis() as i64 + 10 * 60_000;
/// 健康页：超过这么多小时没有一次干净的检查算降级。
pub(super) const FRESHNESS_HOURS: i64 = 72;
/// 同上，换成毫秒；判断和正文都从 `FRESHNESS_HOURS` 来。
pub(super) const FRESHNESS_MS: i64 = FRESHNESS_HOURS * HOUR_MS;
/// 健康页：首轮之后，一组超过这么多天没读干净算降级（一圈 30 天，留 2 天余量）。
pub(super) const COVERAGE_DAYS: i64 = 32;
/// 同上，换成毫秒；判断和正文都从 `COVERAGE_DAYS` 来。
pub(super) const COVERAGE_MS: i64 = COVERAGE_DAYS * 24 * HOUR_MS;

/// 有原意图、还没有结果的一次检查。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct OpenCheck {
    pub key: CheckKey,
    /// 原意图落账的时刻。
    pub since_ms: i64,
}

/// 一组最近一次有结论的结果（没做完的不算结论）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Conclusive {
    Clean,
    Problem {
        at_ms: i64,
        errors: u64,
        broken_packs: u64,
    },
}

/// 当前仓库里一组的累计事实。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct BucketState {
    /// 最近一次读干净的时刻。
    pub last_clean_ms: Option<i64>,
    /// 最近一次有结论的结果。
    pub conclusive: Option<Conclusive>,
    /// 最近一次结果（含没做完）落账的时刻。
    pub latest_outcome_ms: Option<i64>,
    /// 从最近一次干净以来连续不干净（查出问题或没做完）的次数。
    pub non_clean_streak: u32,
    /// 从最近一次干净以来查出问题的次数；没做完的不算也不打断。
    pub problem_streak: u32,
}

/// 最近一次结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LatestCheck {
    pub key: CheckKey,
    pub outcome: CheckOutcome,
    pub at_ms: i64,
}

/// 账本里抽查事件的累计视图。
#[derive(Default)]
pub(super) struct Checks {
    highest: u64,
    first_intent_ms: Option<i64>,
    latest_record_ms: Option<i64>,
    open: BTreeMap<u64, OpenCheck>,
    last_rotation: Option<(CheckBucket, i64)>,
    last_retry_ms: Option<i64>,
    repository: Option<RepositoryDigest>,
    repository_since_ms: Option<i64>,
    buckets: BTreeMap<CheckBucket, BucketState>,
    latest: Option<LatestCheck>,
}

impl Checks {
    /// 按账本顺序喂一条记录；不是抽查事件、或和固定写法对不上的忽略。
    pub(super) fn absorb(&mut self, target: BackupTargetKind, record: &AuditRecord) {
        let Some(event) = read_check_event(record.event()) else {
            return;
        };
        let at = record.recorded_at().as_millisecond();
        self.latest_record_ms = Some(self.latest_record_ms.map_or(at, |latest| latest.max(at)));
        match event {
            CheckEvent::Intent {
                assignment,
                target: claimed,
            } => {
                let sequence = assignment.key.sequence.get();
                // 别的目标类别的检查也占序号：事件键里没有目标，序号必须全账本唯一。
                self.highest = self.highest.max(sequence);
                if claimed != target {
                    return;
                }
                self.first_intent_ms.get_or_insert(at);
                self.open.insert(
                    sequence,
                    OpenCheck {
                        key: assignment.key,
                        since_ms: at,
                    },
                );
                match assignment.purpose {
                    CheckPurpose::Rotation => {
                        self.last_rotation = Some((assignment.key.bucket, at))
                    }
                    CheckPurpose::Retry => self.last_retry_ms = Some(at),
                }
            }
            CheckEvent::Outcome {
                key,
                outcome,
                recorder,
            } => {
                // 只认收口了这里的原意图的结果。
                if self
                    .open
                    .get(&key.sequence.get())
                    .is_none_or(|open| open.key != key)
                {
                    return;
                }
                self.open.remove(&key.sequence.get());
                if let CheckRecorder::Worker(digest) = recorder {
                    self.switch_repository(digest, at);
                }
                self.close(key.bucket, outcome, at);
                self.latest = Some(LatestCheck {
                    key,
                    outcome,
                    at_ms: at,
                });
            }
        }
    }

    /// worker 报的仓库摘要变了：旧仓库各组的结果不再算，覆盖期从这一刻重新起算。
    fn switch_repository(&mut self, digest: RepositoryDigest, at: i64) {
        if self.repository.as_ref() == Some(&digest) {
            return;
        }
        if self.repository.is_some() {
            self.buckets.clear();
        }
        self.repository = Some(digest);
        self.repository_since_ms = Some(at);
    }

    fn close(&mut self, bucket: CheckBucket, outcome: CheckOutcome, at: i64) {
        let state = self.buckets.entry(bucket).or_default();
        state.latest_outcome_ms = Some(at);
        match outcome.result() {
            CheckResult::Clean => {
                state.last_clean_ms = Some(at);
                state.conclusive = Some(Conclusive::Clean);
                state.non_clean_streak = 0;
                state.problem_streak = 0;
            }
            CheckResult::Problem => {
                state.conclusive = Some(Conclusive::Problem {
                    at_ms: at,
                    errors: outcome.errors(),
                    broken_packs: outcome.broken_packs(),
                });
                state.non_clean_streak = state.non_clean_streak.saturating_add(1);
                state.problem_streak = state.problem_streak.saturating_add(1);
            }
            CheckResult::Incomplete => {
                state.non_clean_streak = state.non_clean_streak.saturating_add(1);
            }
        }
    }

    /// 下一个检查序号；单调递增，不复用，和备份作业序号分开。
    pub(super) fn next_sequence(&self) -> Option<NonZeroU64> {
        self.highest.checked_add(1).and_then(NonZeroU64::new)
    }

    /// 挂满 `ABANDON_AFTER_MS` 的原意图：worker 中途没了，要补「没做完」。
    pub(super) fn abandoned(&self, now_ms: i64) -> Vec<CheckKey> {
        self.open
            .values()
            .filter(|open| now_ms.saturating_sub(open.since_ms) >= ABANDON_AFTER_MS)
            .map(|open| open.key)
            .collect()
    }

    /// 还在查的那次：挂着、还没挂满 `ABANDON_AFTER_MS` 的原意图（时钟早于它也算还在查）。
    pub(super) fn running(&self, now_ms: i64) -> Option<OpenCheck> {
        self.open
            .values()
            .find(|open| now_ms.saturating_sub(open.since_ms) < ABANDON_AFTER_MS)
            .copied()
    }

    /// 到期就给组号和原因；没到期、或本地时钟早于账本里的抽查时刻，都没有。
    pub(super) fn due(&self, now_ms: i64) -> Option<(CheckBucket, CheckPurpose)> {
        if self.latest_record_ms.is_some_and(|latest| now_ms < latest) {
            return None;
        }
        if let Some(bucket) = self.retry_due(now_ms) {
            return Some((bucket, CheckPurpose::Retry));
        }
        match self.last_rotation {
            None => Some((CheckBucket::FIRST, CheckPurpose::Rotation)),
            Some((bucket, at)) => (now_ms.saturating_sub(at) >= ROTATION_INTERVAL_MS)
                .then_some((bucket.following(), CheckPurpose::Rotation)),
        }
    }

    /// 到期要重查的组：结果最早的那组先查，同一时刻组号小的先查。
    fn retry_due(&self, now_ms: i64) -> Option<CheckBucket> {
        if self
            .last_retry_ms
            .is_some_and(|at| now_ms.saturating_sub(at) < RETRY_SPACING_MS)
        {
            return None;
        }
        self.buckets
            .iter()
            .filter(|(_, state)| state.non_clean_streak > 0)
            .filter_map(|(bucket, state)| {
                let at = state.latest_outcome_ms?;
                let wait = if state.non_clean_streak == 1 {
                    FIRST_RETRY_AFTER_MS
                } else {
                    REPEAT_RETRY_AFTER_MS
                };
                (now_ms.saturating_sub(at) >= wait).then_some((at, *bucket))
            })
            .min()
            .map(|(_, bucket)| bucket)
    }

    /// 账本里第一次抽查原意图落账的时刻。
    pub(super) fn first_intent_ms(&self) -> Option<i64> {
        self.first_intent_ms
    }

    /// 账本里抽查事件最晚的时刻。
    pub(super) fn latest_record_ms(&self) -> Option<i64> {
        self.latest_record_ms
    }

    /// 当前仓库第一次有 worker 结果的时刻。
    pub(super) fn repository_since_ms(&self) -> Option<i64> {
        self.repository_since_ms
    }

    /// 当前仓库各组的事实；没查过的组没有条目。
    pub(super) fn buckets(&self) -> &BTreeMap<CheckBucket, BucketState> {
        &self.buckets
    }

    /// 最近一次结果。
    pub(super) fn latest(&self) -> Option<LatestCheck> {
        self.latest
    }
}

#[cfg(test)]
#[path = "checks_tests.rs"]
mod tests;
