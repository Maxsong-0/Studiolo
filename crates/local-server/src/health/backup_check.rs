//! 「备份仓库检查」一节：只搬运生产者从账本的抽查事件算出的事实，结论见 `RepositoryChecks::verdict`。
//!
//! 「查出问题」只在读到损坏的数据包时才说损坏；只有错误数时说检查没通过，可能是仓库坏了，
//! 也可能是连不上或凭据不对（restic 分不出来）。正文只有组号、个数、时长和固定中文，不含路径、
//! 主机或 restic 原文，且不出现斜杠（脱敏边界不许）。
use super::backup::span;
use crate::backup::{
    CheckMark, CheckProblem, CheckVerdict, RepositoryCheckEvidence, RepositoryChecks,
};
use studiolo_backup::worker::repository_check::{
    CHECK_BUCKETS, CHECK_WINDOW, CheckResult, IncompleteReason,
};
use studiolo_observability::health::{HealthSection, HealthState};

/// 本节的稳定机器码。
pub const BACKUP_REPOSITORY_CHECK_CODE: &str = "backup-repository-check";
const TITLE: &str = "备份仓库检查";
/// 正文里最多列出几个组号。
const LISTED: usize = 8;

/// 本节结论；正文越出脱敏边界只可能是实现缺陷，退成未知。
pub(super) fn check_section(evidence: &RepositoryCheckEvidence) -> HealthSection {
    let (state, detail) = match evidence {
        RepositoryCheckEvidence::Disabled => (
            HealthState::Unknown,
            "备份生产者没有启用：私有配置里没有备份那两段，看不到仓库检查".to_owned(),
        ),
        RepositoryCheckEvidence::Unreadable => (
            HealthState::Unknown,
            "这一次没读到账本里的仓库检查事件，状态未知".to_owned(),
        ),
        RepositoryCheckEvidence::Read(checks) => (state(checks.verdict()), detail(checks)),
    };
    HealthSection::new(BACKUP_REPOSITORY_CHECK_CODE, TITLE, state, detail)
        .or_else(|_| {
            HealthSection::new(
                BACKUP_REPOSITORY_CHECK_CODE,
                TITLE,
                HealthState::Unknown,
                "仓库检查的结论没能写成脱敏正文，按未知处理",
            )
        })
        .expect("固定健康文案必须满足脱敏契约")
}

fn state(verdict: CheckVerdict) -> HealthState {
    match verdict {
        CheckVerdict::Problem | CheckVerdict::Stale | CheckVerdict::CoverageOverdue => {
            HealthState::Degraded
        }
        CheckVerdict::ClockBehind | CheckVerdict::NotYet => HealthState::Unknown,
        CheckVerdict::Healthy => HealthState::Normal,
    }
}

fn detail(checks: &RepositoryChecks) -> String {
    // 「多久之前」；时刻晚于本地时钟（时钟早于账本）时直说，不编时长。
    let ago = |at_ms: i64| {
        u64::try_from(checks.now_ms.saturating_sub(at_ms)).map_or_else(
            |_| "在晚于本地时钟的时刻".to_owned(),
            |gap| format!("{}前", span(gap)),
        )
    };
    let running = checks.running.map_or_else(String::new, |running| {
        format!("；第 {} 组正在查", running.bucket)
    });
    let (hours, days) = (
        RepositoryChecks::FRESHNESS_HOURS,
        RepositoryChecks::COVERAGE_DAYS,
    );
    match checks.verdict() {
        CheckVerdict::Problem => problem_text(checks, &ago),
        CheckVerdict::Stale => {
            let base = match checks.last_clean_ms() {
                Some(at) => format!("最近一次干净的检查是 {}，超过 {hours} 小时", ago(at)),
                None => format!(
                    "从开始服务或第一次尝试算起已经 {}，还没有一次干净的检查",
                    checks
                        .since_clean_ms()
                        .map_or_else(|| "未知时长".to_owned(), span)
                ),
            };
            format!("{base}{}{running}", latest_text(checks.latest, &ago))
        }
        CheckVerdict::CoverageOverdue => {
            let overdue = checks.overdue_buckets();
            format!(
                "首轮之后每组 {days} 天内都要读一遍：{}超过 {days} 天没读干净",
                bucket_list(&overdue)
            )
        }
        CheckVerdict::ClockBehind => {
            "本地时钟早于账本里的检查时刻，或者读不到起算时刻，判断不了".to_owned()
        }
        CheckVerdict::NotYet => {
            let left = checks
                .freshness_left_ms()
                .map_or_else(|| "未知时长".to_owned(), span);
            format!(
                "还没有干净的检查，离 {hours} 小时的期限还有 {left}{}{running}",
                latest_text(checks.latest, &ago)
            )
        }
        CheckVerdict::Healthy => {
            let last = checks
                .last_clean_ms()
                .map_or_else(|| "未知时刻".to_owned(), ago);
            let coverage = if checks.first_cycle_done() == Some(true) {
                format!(
                    "{days} 天内 {CHECK_BUCKETS} 组都读过，最久的一组 {}前",
                    checks
                        .oldest_clean_ms()
                        .map_or_else(|| "未知时长".to_owned(), span)
                )
            } else {
                format!(
                    "首轮还在读，{CHECK_BUCKETS} 组里读过 {} 组",
                    checks.fresh_buckets()
                )
            };
            format!("最近一次干净的检查是 {last}；{coverage}{running}")
        }
    }
}

/// 先说读到损坏数据包的组；同一组重查干净之前一直显示。
fn problem_text(checks: &RepositoryChecks, ago: &dyn Fn(i64) -> String) -> String {
    let Some(first) = checks
        .problems
        .iter()
        .find(|problem| problem.broken_packs > 0)
        .or_else(|| checks.problems.first())
    else {
        return "有组检查没通过".to_owned();
    };
    let CheckProblem {
        bucket,
        at_ms,
        errors,
        broken_packs,
        times,
    } = *first;
    let what = if broken_packs > 0 {
        format!("读到 {broken_packs} 个损坏的数据包，要用管理员身份按维护步骤修")
    } else {
        format!("检查没通过（报错 {errors} 处），可能是仓库损坏，也可能是连不上或凭据不对")
    };
    let repeated = if times >= 2 {
        format!("，从上次干净以来第 {times} 次")
    } else {
        String::new()
    };
    let others = checks.problems.len() - 1;
    let more = if others > 0 {
        format!("；另有 {others} 组也没通过")
    } else {
        String::new()
    };
    format!(
        "第 {bucket} 组 {}{what}{repeated}{more}；同一组重查干净前一直显示",
        ago(at_ms)
    )
}

/// 最近一次结果的补充说明。
fn latest_text(latest: Option<CheckMark>, ago: &dyn Fn(i64) -> String) -> String {
    let Some(mark) = latest else {
        return String::new();
    };
    let result = match mark.result {
        CheckResult::Clean => "干净".to_owned(),
        CheckResult::Problem => "没通过".to_owned(),
        CheckResult::Incomplete => format!("没做完（{}）", reason_text(mark.reason)),
    };
    format!(
        "；最近一次是 {}第 {} 组，{result}",
        ago(mark.at_ms),
        mark.bucket
    )
}

fn reason_text(reason: Option<IncompleteReason>) -> String {
    match reason {
        Some(IncompleteReason::Preempted) => "让给了备份作业".to_owned(),
        Some(IncompleteReason::Deadline) => format!("超过 {} 分钟", CHECK_WINDOW.as_secs() / 60),
        Some(IncompleteReason::OutputLimit) => "输出超过上限".to_owned(),
        Some(IncompleteReason::Protocol) => "输出和固定版本对不上".to_owned(),
        Some(IncompleteReason::Unavailable) => "没装配起来".to_owned(),
        Some(IncompleteReason::Abandoned) => "worker 中途没了".to_owned(),
        None => "原因不明".to_owned(),
    }
}

/// 「第 7、10 组」；太多只列前几个。
fn bucket_list(buckets: &[u8]) -> String {
    let shown: Vec<String> = buckets
        .iter()
        .take(LISTED)
        .map(ToString::to_string)
        .collect();
    let rest = buckets.len().saturating_sub(LISTED);
    if rest > 0 {
        format!("第 {} 组等 {} 组", shown.join("、"), buckets.len())
    } else {
        format!("第 {} 组", shown.join("、"))
    }
}
