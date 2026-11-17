//! 仓库每日轮流抽查的执行器：只跑生产者派下的那一组 `check --no-lock --read-data-subset n/30`。
//!
//! 不碰正文和封存，也不需要作业准入；固定包、凭据和进程边界和作业共用 [`Runtime`]。一次最多
//! 20 分钟（共用包的 `CHECK_WINDOW`），`interrupt` 报有作业进队就停下让路。所有失败都归成「没做完」
//! 交回生产者落账，只有子进程清理没能证实时不上报、由调用方让整个 worker 退出。
use crate::{ProcessError, ResticCredentials, WorkerConfig, runtime::Runtime};
use std::time::Instant;
use studiolo_backup::worker::{
    repository_check::{CheckAssignment, CheckOutcome, IncompleteReason, classify_check_output},
    restic::{CredentialHolder, ResticCommand, ResticIdentity, ResticInvocation},
};
use studiolo_process_safety::Residency;

/// 跑一次抽查的结局。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Checked {
    /// 有结论或没做完，都要交回生产者落账。
    Outcome(CheckOutcome),
    /// 子进程清理没能证实：不上报，整个 worker 退出。
    Shutdown,
}

/// 跑生产者派下的那一组；凭据从固定系统凭据目录现读，用完即丢。
pub(crate) fn run(
    config: &WorkerConfig,
    residency: &Residency,
    assignment: &CheckAssignment,
    interrupt: &dyn Fn() -> crate::Result<()>,
) -> Checked {
    // 写全路径：生产者判断「中途没了」的时限也从这个常量算，本地同名常量顶替不了它。
    let deadline = Instant::now() + studiolo_backup::worker::repository_check::CHECK_WINDOW;
    let output = ResticCredentials::load(residency, config, deadline)
        .and_then(|credentials| Runtime::open(config, credentials, deadline))
        .and_then(|runtime| {
            let invocation = ResticInvocation::plan(
                runtime.repository().clone(),
                &ResticCommand::CheckSubset {
                    bucket: assignment.key.bucket,
                },
                CredentialHolder::BackupWorker,
                ResticIdentity::Append,
            )
            .map_err(|_| ProcessError::Configuration)?;
            runtime.check_subset(invocation.argv(), interrupt)
        });
    match output {
        Ok(output) => Checked::Outcome(classify_check_output(output.success, &output.stdout)),
        Err(error) => settle(error),
    }
}

/// 没跑出 restic 输出时的结局：清理没能证实要退出，其余按原因归成没做完。
pub(crate) fn settle(error: ProcessError) -> Checked {
    let reason = match error {
        ProcessError::CleanupFailed => return Checked::Shutdown,
        ProcessError::Interrupted => IncompleteReason::Preempted,
        ProcessError::Deadline => IncompleteReason::Deadline,
        ProcessError::OutputLimit => IncompleteReason::OutputLimit,
        ProcessError::Protocol => IncompleteReason::Protocol,
        ProcessError::Package
        | ProcessError::Configuration
        | ProcessError::Source
        | ProcessError::Spawn
        | ProcessError::Io => IncompleteReason::Unavailable,
    };
    Checked::Outcome(CheckOutcome::incomplete(reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_failure_but_unproven_cleanup_becomes_an_incomplete_outcome() {
        for (error, reason) in [
            (ProcessError::Interrupted, IncompleteReason::Preempted),
            (ProcessError::Deadline, IncompleteReason::Deadline),
            (ProcessError::OutputLimit, IncompleteReason::OutputLimit),
            (ProcessError::Protocol, IncompleteReason::Protocol),
            (ProcessError::Package, IncompleteReason::Unavailable),
            (ProcessError::Configuration, IncompleteReason::Unavailable),
            (ProcessError::Source, IncompleteReason::Unavailable),
            (ProcessError::Spawn, IncompleteReason::Unavailable),
            (ProcessError::Io, IncompleteReason::Unavailable),
        ] {
            assert_eq!(
                settle(error),
                Checked::Outcome(CheckOutcome::incomplete(reason)),
                "{error:?}"
            );
        }
        assert_eq!(settle(ProcessError::CleanupFailed), Checked::Shutdown);
    }
}
