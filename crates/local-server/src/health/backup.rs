//! 「备份作业」一节：只搬运生产者从原账本算出的作业事实，阈值来自固定配置。
//!
//! 结论和明细出自同一份 [`BackupJobs`]：卡住、逾期、最近失败是降级，判断不了、还没成功过是未知，
//! 只有「有成功、不逾期、没卡住、最近一次也成功」才是正常。正文只有序号、时长和固定中文，
//! 不含路径、主机或原文。
use crate::backup::{BackupJobs, BackupJobsEvidence, JobFailure, JobsVerdict};
use serde::Serialize;
use studiolo_backup::worker::{restic::UnverifiedReason, state::FailureReason};
use studiolo_observability::health::{HealthSection, HealthState};

/// 本节的稳定机器码。
pub const BACKUP_JOBS_CODE: &str = "backup-jobs";
const TITLE: &str = "备份作业";

/// 最近一次成功的作业；u64 按十进制字符串传输。
#[derive(Debug, Serialize)]
pub struct BackupMarkBody {
    pub(super) sequence: String,
    pub(super) at: Option<String>,
}

/// 还没收口的最早那条作业。
#[derive(Debug, Serialize)]
pub struct BackupOpenBody {
    pub(super) sequence: String,
    pub(super) since: Option<String>,
    /// 已经封存，等 worker 上传。
    pub(super) admitted: bool,
    /// 判断不了时为 `null`。
    pub(super) stuck: Option<bool>,
}

/// 失败原因的固定机器码，和账本里的写法一致。
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum BackupReasonBody {
    /// 明确失败的原因。
    Failed(FailureReason),
    /// 明确未验证的原因。
    Unverified(UnverifiedReason),
}

/// 最近一次没成功的作业。
#[derive(Debug, Serialize)]
pub struct BackupFailureBody {
    pub(super) sequence: String,
    pub(super) at: Option<String>,
    /// `failed`、`unverified` 或 `unrecognized`。
    pub(super) result: &'static str,
    /// 认不出的结果没有原因。
    pub(super) reason: Option<BackupReasonBody>,
}

/// 「备份作业」一节的明细；和本节结论出自同一次读账本。
#[derive(Debug, Serialize)]
pub struct BackupJobsBody {
    pub(super) state: &'static str,
    pub(super) verdict: &'static str,
    pub(super) as_of: Option<String>,
    pub(super) last_success: Option<BackupMarkBody>,
    pub(super) outstanding: Option<BackupOpenBody>,
    pub(super) overdue: Option<bool>,
    /// 逾期从哪一刻起算：最近一次成功，或者从没成功过时开始服务和第一次尝试里较早的那个。
    pub(super) overdue_since: Option<String>,
    pub(super) latest_failed: bool,
    pub(super) last_failure: Option<BackupFailureBody>,
    pub(super) stuck_after_seconds: String,
    pub(super) overdue_after_seconds: String,
}

impl BackupJobsBody {
    pub(super) fn new(jobs: &BackupJobs) -> Self {
        let verdict = jobs.verdict();
        Self {
            state: state(verdict).code(),
            verdict: verdict_code(verdict),
            as_of: timestamp(jobs.now_ms),
            last_success: jobs.last_success.map(|mark| BackupMarkBody {
                sequence: mark.sequence.to_string(),
                at: timestamp(mark.at_ms),
            }),
            outstanding: jobs.outstanding.map(|open| BackupOpenBody {
                sequence: open.sequence.to_string(),
                since: timestamp(open.since_ms),
                admitted: open.admitted,
                stuck: jobs.stuck(),
            }),
            overdue: jobs.overdue(),
            overdue_since: jobs.overdue_since_ms().and_then(timestamp),
            latest_failed: jobs.latest_failed,
            last_failure: jobs.last_failure.map(|mark| BackupFailureBody {
                sequence: mark.sequence.to_string(),
                at: timestamp(mark.at_ms),
                result: match mark.failure {
                    JobFailure::Failed(_) => "failed",
                    JobFailure::Unverified(_) => "unverified",
                    JobFailure::Unrecognized => "unrecognized",
                },
                reason: match mark.failure {
                    JobFailure::Failed(reason) => Some(BackupReasonBody::Failed(reason)),
                    JobFailure::Unverified(reason) => Some(BackupReasonBody::Unverified(reason)),
                    JobFailure::Unrecognized => None,
                },
            }),
            stuck_after_seconds: (jobs.stuck_after_ms / 1_000).to_string(),
            overdue_after_seconds: (jobs.overdue_after_ms / 1_000).to_string(),
        }
    }
}

/// 本节结论；正文越出脱敏边界只可能是实现缺陷，退成未知。
pub(super) fn jobs_section(evidence: &BackupJobsEvidence) -> HealthSection {
    let (state, detail) = match evidence {
        BackupJobsEvidence::Disabled => (
            HealthState::Unknown,
            "备份生产者没有启用：私有配置里没有备份那两段，看不到作业".to_owned(),
        ),
        BackupJobsEvidence::Unreadable => (
            HealthState::Unknown,
            "这一次没读到账本里的备份事件，作业状态未知".to_owned(),
        ),
        BackupJobsEvidence::Read(jobs) => (state(jobs.verdict()), detail(jobs)),
    };
    HealthSection::new(BACKUP_JOBS_CODE, TITLE, state, detail)
        .or_else(|_| {
            HealthSection::new(
                BACKUP_JOBS_CODE,
                TITLE,
                HealthState::Unknown,
                "备份作业的结论没能写成脱敏正文，按未知处理",
            )
        })
        .expect("固定健康文案必须满足脱敏契约")
}

fn state(verdict: JobsVerdict) -> HealthState {
    match verdict {
        JobsVerdict::Stuck | JobsVerdict::Overdue | JobsVerdict::LatestFailed => {
            HealthState::Degraded
        }
        JobsVerdict::ClockBehind | JobsVerdict::NoSuccessYet => HealthState::Unknown,
        JobsVerdict::Healthy => HealthState::Normal,
    }
}

fn verdict_code(verdict: JobsVerdict) -> &'static str {
    match verdict {
        JobsVerdict::Stuck => "stuck",
        JobsVerdict::Overdue => "overdue",
        JobsVerdict::LatestFailed => "latest_failed",
        JobsVerdict::ClockBehind => "clock_behind",
        JobsVerdict::NoSuccessYet => "no_success_yet",
        JobsVerdict::Healthy => "healthy",
    }
}

fn detail(jobs: &BackupJobs) -> String {
    let since_success = jobs
        .since_success_ms()
        .map_or_else(|| "未知".to_owned(), span);
    let overdue_line = span(jobs.overdue_after_ms);
    match jobs.verdict() {
        JobsVerdict::Stuck => {
            let open = jobs.outstanding.map_or(0, |open| open.sequence);
            let held = jobs
                .outstanding_for_ms()
                .map_or_else(|| "未知".to_owned(), span);
            format!(
                "第 {open} 号作业挂了 {held}还没收口，超过保留期 {}，算卡住",
                span(jobs.stuck_after_ms)
            )
        }
        JobsVerdict::Overdue => match jobs.last_success {
            Some(mark) => format!(
                "最近一次成功是 {since_success}前的第 {} 号，超过两倍最小间隔 {overdue_line}，算逾期",
                mark.sequence
            ),
            None => format!(
                "从开始服务或第一次尝试算起已经 {since_success}，一次成功都没有，超过两倍最小间隔 {overdue_line}，算逾期"
            ),
        },
        JobsVerdict::LatestFailed => {
            let (sequence, why) = jobs.last_failure.map_or((0, "结果读不出来"), |mark| {
                (mark.sequence, failure_text(mark.failure))
            });
            format!("最近收口的第 {sequence} 号作业没成功：{why}；按批准的间隔重试")
        }
        JobsVerdict::ClockBehind => {
            "本地时钟早于账本里的备份时刻，或者读不到起算时刻，判断不了卡住和逾期".to_owned()
        }
        JobsVerdict::NoSuccessYet => {
            let left = jobs.since_success_ms().map_or_else(
                || "未知".to_owned(),
                |gap| span(jobs.overdue_after_ms.saturating_sub(gap)),
            );
            format!("还没有成功的备份，离逾期线还有 {left}")
        }
        JobsVerdict::Healthy => {
            let sequence = jobs.last_success.map_or(0, |mark| mark.sequence);
            match (jobs.outstanding, jobs.outstanding_for_ms()) {
                (Some(open), Some(held)) => format!(
                    "最近一次成功是 {since_success}前的第 {sequence} 号；第 {} 号作业在做，已挂 {}",
                    open.sequence,
                    span(held)
                ),
                _ => format!("最近一次成功是 {since_success}前的第 {sequence} 号"),
            }
        }
    }
}

/// 失败原因的中文说法。
fn failure_text(failure: JobFailure) -> &'static str {
    match failure {
        JobFailure::Failed(reason) => match reason {
            FailureReason::BarrierNotEstablished => {
                "冻结、导出或准入没做成（跨 owner 屏障没有建立）"
            }
            FailureReason::SourceChangedBeforeSeal => "封存前来源变了",
            FailureReason::OutOfSpace => "磁盘空间不够",
            FailureReason::UploadRejected => "上传被拒绝",
            FailureReason::RemoteVerificationFailed => "远端校验没通过",
            FailureReason::FinalManifestCommitFailed => "最终清单没提交成",
            FailureReason::TargetUnverified => "备份目标没接入，验证不了",
            FailureReason::SourceExpired => "封存正文过了保留期，作业没做完",
        },
        JobFailure::Unverified(reason) => match reason {
            UnverifiedReason::NoRealRepository => "未验证：还没有真实的备份仓库",
            UnverifiedReason::TargetUnreachable => "未验证：备份目标连不上",
            UnverifiedReason::ImmutabilityUnprovable => "未验证：证明不了目标的不可变保护",
        },
        JobFailure::Unrecognized => "结果读不出来，不当成功",
    }
}

/// 时长的中文说法：一天以上精确到小时，一天以内精确到分钟，不到一分钟单独说。
pub(super) fn span(millis: u64) -> String {
    let minutes = millis / 60_000;
    let (days, hours, rest) = (minutes / 1_440, minutes / 60 % 24, minutes % 60);
    match (days, hours, rest) {
        (0, 0, 0) => "不到 1 分钟".to_owned(),
        (0, 0, rest) => format!("{rest} 分钟"),
        (0, hours, 0) => format!("{hours} 小时"),
        (0, hours, rest) => format!("{hours} 小时 {rest} 分钟"),
        (days, 0, _) => format!("{days} 天"),
        (days, hours, _) => format!("{days} 天 {hours} 小时"),
    }
}

/// Unix 毫秒转成 RFC 3339；不是正数或越界就是 `None`，前端显示未知。
fn timestamp(millis: i64) -> Option<String> {
    if millis <= 0 {
        return None;
    }
    jiff::Timestamp::from_millisecond(millis)
        .ok()
        .map(|value| value.to_string())
}
