//! 健康页「备份仓库检查」一节的事实和结论：生产者从账本的抽查事件算出事实，这里只判。
//!
//! 降级：有组最近一次有结论的结果是查出问题（一次就算，直到同一组重查干净）；超过 72 小时没有
//! 一次干净的检查；首轮之后有组超过 32 天没读干净。未知：本地时钟早于账本、还没有干净的检查且
//! 不满 72 小时。其余正常，首轮里也正常，正文写明读了几组。
//!
//! 本地时钟早于账本时，算出来的间隔只会偏短：据此判出的过期、覆盖逾期一定成立，照样报降级；
//! 判不出问题时才说未知，不说正常。
use super::checks::{
    COVERAGE_DAYS, COVERAGE_MS, Checks, Conclusive, FRESHNESS_HOURS, FRESHNESS_MS,
};
use studiolo_backup::worker::repository_check::{CHECK_BUCKETS, CheckResult, IncompleteReason};

const BUCKETS: usize = CHECK_BUCKETS as usize;

/// 一组最近一次有结论的结果是查出问题。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckProblem {
    /// 组号。
    pub bucket: u8,
    /// 这次结果落账的时刻（Unix 毫秒）。
    pub at_ms: i64,
    /// summary 里的错误数。
    pub errors: u64,
    /// 损坏的数据包个数；大于 0 才说明读到了损坏的数据包。
    pub broken_packs: u64,
    /// 从这一组最近一次干净以来查出问题的次数。
    pub times: u32,
}

/// 最近一次结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckMark {
    /// 组号。
    pub bucket: u8,
    /// 结果落账的时刻（Unix 毫秒）。
    pub at_ms: i64,
    /// 三类结果。
    pub result: CheckResult,
    /// 没做完的原因。
    pub reason: Option<IncompleteReason>,
}

/// 正在查的那次。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunningCheck {
    /// 组号。
    pub bucket: u8,
    /// 原意图落账的时刻（Unix 毫秒）。
    pub since_ms: i64,
}

/// 「备份仓库检查」一节要的事实。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryChecks {
    /// 本次判断用的本地时刻（Unix 毫秒）。
    pub now_ms: i64,
    /// 还没有干净的检查时「多久没查成」的起算点：开始服务、第一次备份尝试、第一次抽查原意图里
    /// 最早的那个，所以重启不会把一直没查成重新算成刚开始。
    pub anchor_ms: Option<i64>,
    /// 当前仓库第一次有结果的时刻；首轮覆盖期从它起算。
    pub repository_since_ms: Option<i64>,
    /// 账本里抽查事件最晚的时刻。
    pub latest_record_ms: Option<i64>,
    /// 正在查的那次。
    pub running: Option<RunningCheck>,
    /// 最近一次结果。
    pub latest: Option<CheckMark>,
    /// 当前仓库各组最近一次读干净的时刻，下标是组号减一。
    pub last_clean_ms: [Option<i64>; BUCKETS],
    /// 最近一次有结论的结果是查出问题的组，按组号排。
    pub problems: Vec<CheckProblem>,
}

/// 结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckVerdict {
    /// 有组最近一次有结论的结果是查出问题。
    Problem,
    /// 超过 72 小时没有一次干净的检查（还没有过就从起算点算）。
    Stale,
    /// 首轮之后有组超过 32 天没读干净。
    CoverageOverdue,
    /// 本地时钟早于账本里的抽查时刻，或读不到起算点；能证实的过期、覆盖逾期排在它前面。
    ClockBehind,
    /// 还没有干净的检查，也还没到 72 小时。
    NotYet,
    /// 有干净的检查、不过期、没有查出问题的组、首轮之后每组 32 天内都读过。
    Healthy,
}

impl RepositoryChecks {
    /// 超过多少小时没有一次干净的检查算降级；正文和判断用同一个常量。
    pub const FRESHNESS_HOURS: i64 = FRESHNESS_HOURS;
    /// 首轮之后，一组超过多少天没读干净算降级；正文和判断用同一个常量。
    pub const COVERAGE_DAYS: i64 = COVERAGE_DAYS;

    /// 由生产者的账本视图算出；`anchor_ms` 由生产者从开始服务和账本里的第一次尝试算好传进来。
    pub(super) fn from_view(now_ms: i64, anchor_ms: Option<i64>, checks: &Checks) -> Self {
        let mut last_clean_ms = [None; BUCKETS];
        let mut problems = Vec::new();
        for (bucket, state) in checks.buckets() {
            last_clean_ms[usize::from(bucket.get() - 1)] = state.last_clean_ms;
            if let Some(Conclusive::Problem {
                at_ms,
                errors,
                broken_packs,
            }) = state.conclusive
            {
                problems.push(CheckProblem {
                    bucket: bucket.get(),
                    at_ms,
                    errors,
                    broken_packs,
                    times: state.problem_streak,
                });
            }
        }
        Self {
            now_ms,
            anchor_ms,
            repository_since_ms: checks.repository_since_ms(),
            latest_record_ms: checks.latest_record_ms(),
            running: checks.running(now_ms).map(|open| RunningCheck {
                bucket: open.key.bucket.get(),
                since_ms: open.since_ms,
            }),
            latest: checks.latest().map(|latest| CheckMark {
                bucket: latest.key.bucket.get(),
                at_ms: latest.at_ms,
                result: latest.outcome.result(),
                reason: latest.outcome.reason(),
            }),
            last_clean_ms,
            problems,
        }
    }

    /// 最近一次干净的检查（任何一组）。
    #[must_use]
    pub fn last_clean_ms(&self) -> Option<i64> {
        self.last_clean_ms.iter().flatten().max().copied()
    }

    /// 离最近一次干净的检查（没有就是起算点）过了多久；时钟早于它、读不到起算点为 `None`。
    #[must_use]
    pub fn since_clean_ms(&self) -> Option<u64> {
        elapsed(self.now_ms, self.last_clean_ms().or(self.anchor_ms)?)
    }

    /// 离「超过 72 小时没有干净的检查」还剩多久；时钟早于起算点、读不到起算点为 `None`。
    #[must_use]
    pub fn freshness_left_ms(&self) -> Option<u64> {
        self.since_clean_ms()
            .map(|gap| span(FRESHNESS_MS).saturating_sub(gap))
    }

    /// 首轮读完了没有：当前仓库有结果满 32 天。还没有结果、时钟早于它为 `None`。
    #[must_use]
    pub fn first_cycle_done(&self) -> Option<bool> {
        elapsed(self.now_ms, self.repository_since_ms?).map(|gap| gap > span(COVERAGE_MS))
    }

    /// 超过 32 天没读干净（含从没读干净）的组号。读干净的时刻晚于本地时钟的那组判断不了，不算。
    #[must_use]
    pub fn overdue_buckets(&self) -> Vec<u8> {
        (1..=CHECK_BUCKETS)
            .zip(&self.last_clean_ms)
            .filter(|(_, at)| {
                at.is_none_or(|at| {
                    elapsed(self.now_ms, at).is_some_and(|age| age > span(COVERAGE_MS))
                })
            })
            .map(|(bucket, _)| bucket)
            .collect()
    }

    /// 32 天内读干净过几组。
    #[must_use]
    pub fn fresh_buckets(&self) -> usize {
        self.bucket_ages()
            .filter(|(_, age)| age.is_some_and(|age| age <= span(COVERAGE_MS)))
            .count()
    }

    /// 读干净过的组里最久那组读完过了多久。
    #[must_use]
    pub fn oldest_clean_ms(&self) -> Option<u64> {
        self.bucket_ages().filter_map(|(_, age)| age).max()
    }

    /// 已知的问题排在判断不了前面。本地时钟早于账本时算出的间隔只会偏短，所以按它判出的过期、
    /// 覆盖逾期一定成立；判不出问题时才因为时钟说未知，不说正常也不说还没到期。
    #[must_use]
    pub fn verdict(&self) -> CheckVerdict {
        if !self.problems.is_empty() {
            return CheckVerdict::Problem;
        }
        let since_clean = self.since_clean_ms();
        if since_clean.is_some_and(|gap| gap > span(FRESHNESS_MS)) {
            return CheckVerdict::Stale;
        }
        if self.first_cycle_done() == Some(true) && !self.overdue_buckets().is_empty() {
            return CheckVerdict::CoverageOverdue;
        }
        if since_clean.is_none()
            || self
                .latest_record_ms
                .is_some_and(|latest| self.now_ms < latest)
        {
            return CheckVerdict::ClockBehind;
        }
        if self.last_clean_ms().is_none() {
            CheckVerdict::NotYet
        } else {
            CheckVerdict::Healthy
        }
    }

    /// 每组的组号和离最近一次读干净过了多久；没读干净过、时钟早于它为 `None`。
    fn bucket_ages(&self) -> impl Iterator<Item = (u8, Option<u64>)> + '_ {
        (1..=CHECK_BUCKETS)
            .zip(&self.last_clean_ms)
            .map(|(bucket, at)| (bucket, at.and_then(|at| elapsed(self.now_ms, at))))
    }
}

/// 健康页拿到的抽查证据。默认是「没读到」。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RepositoryCheckEvidence {
    /// 生产者没启用。
    Disabled,
    /// 生产者启用了，但这一次没读到。
    #[default]
    Unreadable,
    /// 读到了；每组一个时刻，体积大，装箱免得整个枚举跟着变大。
    Read(Box<RepositoryChecks>),
}

fn span(millis: i64) -> u64 {
    u64::try_from(millis).unwrap_or(u64::MAX)
}

/// 从 `since` 到 `now` 过了多少毫秒；时钟早于 `since` 时为 `None`。
fn elapsed(now: i64, since: i64) -> Option<u64> {
    u64::try_from(now.checked_sub(since)?).ok()
}

#[cfg(test)]
#[path = "check_evidence_tests.rs"]
mod tests;
