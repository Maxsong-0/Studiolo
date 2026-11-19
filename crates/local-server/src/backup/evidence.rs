//! 备份事实的类型和只读探针口；健康页只经 `BackupJobsProbe` 取事实，自己不读账本。
//!
//! 作业事实（`BackupJobs`）由主服务里的生产者从原账本的备份事件算出来，健康页的「备份作业」和
//! 「恢复点年龄」两节都用它：恢复点取成功作业准入里的内容截止，判据和 worker 认「可恢复点」一样。

use studiolo_backup::worker::restic::UnverifiedReason;
use studiolo_backup::worker::state::FailureReason;

/// 一条已收口作业的序号和结果落账的时刻（Unix 毫秒）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JobMark {
    /// 作业序号。
    pub sequence: u64,
    /// 结果落账的时刻。
    pub at_ms: i64,
}

/// 最新的灾难恢复点：成功收口的作业里内容截止最晚的那一条。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryMark {
    /// 作业序号。
    pub sequence: u64,
    /// 这次作业准入里的内容截止时刻（Unix 毫秒）；恢复点年龄从它算，不从上传完成算。
    pub content_cutoff_ms: i64,
}

/// 没成功的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobFailure {
    /// 明确失败，含生产者补的封存失败。
    Failed(FailureReason),
    /// 明确未验证：不是成功，也不算一个恢复点。
    Unverified(UnverifiedReason),
    /// 结果字段读不出来，或者自称成功却没写明对象齐全、校验通过；照样收口，但不当成功。
    Unrecognized,
}

/// 最近一次没成功的作业。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailureMark {
    /// 作业序号。
    pub sequence: u64,
    /// 结果落账的时刻（Unix 毫秒）。
    pub at_ms: i64,
    /// 没成功在哪。
    pub failure: JobFailure,
}

/// 还没收口的最早那条作业。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenJob {
    /// 作业序号。
    pub sequence: u64,
    /// 原意图落账的时刻（Unix 毫秒）。
    pub since_ms: i64,
    /// 已经封存（账本里有准入），等 worker 上传。
    pub admitted: bool,
}

/// 健康页判断备份作业用的事实：全部来自原账本里的备份事件，阈值来自固定配置，请求里没有能改它的参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackupJobs {
    /// 这一次判断用的本地时刻（Unix 毫秒）。
    pub now_ms: i64,
    /// 生产者开始服务的时刻；当时读不到本地时钟为 `None`。
    pub serving_since_ms: Option<i64>,
    /// 账本里第一次尝试（原意图）的时刻。
    pub first_attempt_ms: Option<i64>,
    /// 最近一次成功。
    pub last_success: Option<JobMark>,
    /// 最近一次没成功的作业，不管它之后有没有成功过。
    pub last_failure: Option<FailureMark>,
    /// 最近收口的那条作业没成功。
    pub latest_failed: bool,
    /// 还没收口的最早那条作业。
    pub outstanding: Option<OpenJob>,
    /// 挂得比它还久算卡住：封存正文的保留期，过了它 worker 已经传不了。
    pub stuck_after_ms: u64,
    /// 这么久没有成功算逾期：两倍最小间隔。
    pub overdue_after_ms: u64,
    /// 最新的灾难恢复点；从没有成功过（或成功的都没有准入记录）为 `None`，健康页显示「尚未建立」。
    pub recovery_point: Option<RecoveryMark>,
    /// 恢复点年龄预算：主人批准的 RPO。
    pub recovery_point_budget_ms: u64,
}

/// 备份作业的结论，从重到轻排。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobsVerdict {
    /// 没收口的作业挂得比保留期还久。
    Stuck,
    /// 超过两倍最小间隔没有成功。
    Overdue,
    /// 最近收口的作业没成功。
    LatestFailed,
    /// 本地时钟早于要比的账本时刻，判断不了卡住或逾期。
    ClockBehind,
    /// 还没成功过，也还没到逾期线。
    NoSuccessYet,
    /// 有成功、不逾期、没卡住、最近一次也成功了。
    Healthy,
}

impl BackupJobs {
    /// 没收口的作业挂了多久；没有挂着的、或者时钟早于原意图，都是 `None`。
    #[must_use]
    pub fn outstanding_for_ms(&self) -> Option<u64> {
        self.outstanding
            .and_then(|open| elapsed(self.now_ms, open.since_ms))
    }

    /// 挂着的作业算不算卡住；没有挂着的算不卡，时钟早于原意图判断不了。
    #[must_use]
    pub fn stuck(&self) -> Option<bool> {
        match self.outstanding {
            None => Some(false),
            Some(_) => self
                .outstanding_for_ms()
                .map(|held| held > self.stuck_after_ms),
        }
    }

    /// 逾期的起算点：最近一次成功；从没成功过就取开始服务和第一次尝试里较早的那个，
    /// 所以重启不会把「一直没成功」重新算成刚开始。
    #[must_use]
    pub fn overdue_since_ms(&self) -> Option<i64> {
        match self.last_success {
            Some(mark) => Some(mark.at_ms),
            None => [self.serving_since_ms, self.first_attempt_ms]
                .into_iter()
                .flatten()
                .min(),
        }
    }

    /// 离上一个成功（或起算点）过了多久；起算点读不到、时钟早于它，都是 `None`。
    #[must_use]
    pub fn since_success_ms(&self) -> Option<u64> {
        elapsed(self.now_ms, self.overdue_since_ms()?)
    }

    /// 算不算逾期；判断不了为 `None`。
    #[must_use]
    pub fn overdue(&self) -> Option<bool> {
        self.since_success_ms()
            .map(|gap| gap > self.overdue_after_ms)
    }

    /// 已知的问题排在判断不了前面：卡住、逾期、最近失败都比「时钟不对」更该先看。
    #[must_use]
    pub fn verdict(&self) -> JobsVerdict {
        let (stuck, overdue) = (self.stuck(), self.overdue());
        if stuck == Some(true) {
            JobsVerdict::Stuck
        } else if overdue == Some(true) {
            JobsVerdict::Overdue
        } else if self.latest_failed {
            JobsVerdict::LatestFailed
        } else if stuck.is_none() || overdue.is_none() {
            JobsVerdict::ClockBehind
        } else if self.last_success.is_none() {
            JobsVerdict::NoSuccessYet
        } else {
            JobsVerdict::Healthy
        }
    }
}

/// 从 `since` 到 `now` 过了多少毫秒；时钟早于 `since` 时为 `None`，按判断不了处理。
fn elapsed(now: i64, since: i64) -> Option<u64> {
    u64::try_from(now.checked_sub(since)?).ok()
}

/// 备份作业的只读探针。由持有启动保留原账本的生产者实现；健康页只经它取事实，不写账本、不安排封存。
pub trait BackupJobsProbe: Send + Sync {
    /// 读一次账本里的备份事件，按固定配置给出作业事实；这一次没读完返回 `None`。
    fn jobs(&self, now_ms: i64) -> Option<BackupJobs>;

    /// 读一次账本里的抽查事件，给出「备份仓库检查」一节的事实；这一次没读完返回 `None`。
    /// 只读：不补「没做完」、不派检查。
    fn repository_checks(&self, now_ms: i64) -> Option<super::RepositoryChecks>;
}

/// 健康页拿到的备份作业证据。默认是「没读到」：什么都不知道时不能说成没启用，更不能说成正常。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackupJobsEvidence {
    /// 启动时没有给备份配置（`--backup-config`），生产者没启用。
    Disabled,
    /// 生产者启用了，但这一次没读到：账本没读完、锁坏了或本地时钟不可用。
    #[default]
    Unreadable,
    /// 读到了。
    Read(BackupJobs),
}

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;
