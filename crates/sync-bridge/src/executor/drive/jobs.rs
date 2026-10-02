//! 逐动作执行：保全在前、Issued 在中、回执在后；未知永不补成成功。

use super::{ExecutionPorts, ExecutionReport};
use crate::executor::{
    cas::ObjectId,
    deletion::DeletionClearance,
    error::ExecutorError,
    journal::{JobOutcome, JobRecord, JobState, JournalFault, PlanJournal},
    quiescence::Quiesced,
    transfer::{self, JobBackend},
};
use crate::{Action, Change, FuseReason, Side};

/// BridgeState 单次 flush 的上限。
const AUDIT_BATCH: usize = 64;
/// 有界轮次；到顶说明审计无法推进，绝不无限重试。
const AUDIT_ROUNDS: usize = 512;

/// 安全边界上读取关停请求；不改变任何动作或审计终态。
pub(super) fn check_stopping(backend: &dyn JobBackend) -> Result<(), ExecutorError> {
    if backend.stopping() {
        Err(ExecutorError::Stopping)
    } else {
        Ok(())
    }
}

/// 每个固定大小的审计批次后可安全退出；未确认事件留在持久 outbox。
pub(super) fn drain_audit(
    journal: &mut dyn PlanJournal,
    backend: &dyn JobBackend,
) -> Result<usize, ExecutorError> {
    let mut total = 0_usize;
    for _ in 0..AUDIT_ROUNDS {
        check_stopping(backend)?;
        let progress = journal
            .deliver_audit(AUDIT_BATCH)
            .map_err(ExecutorError::Journal)?;
        total = total.saturating_add(progress.acknowledged);
        if !progress.remaining {
            return Ok(total);
        }
        if progress.acknowledged == 0 {
            return Err(ExecutorError::AuditStalled);
        }
    }
    Err(ExecutorError::AuditStalled)
}

/// 执行一条动作；已有持久状态的原键不重放、不改写。
pub(super) fn run(
    ports: &mut ExecutionPorts<'_>,
    action: &Action,
    record: &JobRecord,
    quiesced: &Quiesced,
    clearance: &DeletionClearance,
    report: &mut ExecutionReport,
) -> Result<(), ExecutorError> {
    match record.state {
        JobState::Committed => {
            report.already_known += 1;
            return Ok(());
        }
        JobState::NoEffect => {
            report.already_known += 1;
            report.no_effect += 1;
            return Ok(());
        }
        // 重开后仍是 Issued：结果未知。不重发、不换键，交由原键回执核对闭合。
        JobState::Issued => {
            report.already_known += 1;
            report.unknown += 1;
            return Ok(());
        }
        JobState::IntentPending => {
            return Err(ExecutorError::Journal(JournalFault::IntentRequired));
        }
        JobState::Reserved => {}
    }
    if matches!(action.change, Change::Delete { .. }) && clearance.deletions() == 0 {
        return Err(ExecutorError::DeletionFuseBlocked(FuseReason::Count));
    }
    let recheck = ports
        .witness
        .recheck(&action.key, quiesced)
        .map_err(ExecutorError::Witness)?;
    if ports
        .journal
        .cancel_changed(&action.key, &recheck)
        .map_err(ExecutorError::Journal)?
    {
        report.no_effect += 1;
        return Ok(());
    }
    // 条件仍有效才保全；保全失败就不会有 Issued，issue 再检查证据截止点。
    let staged = stage(ports, action)?;
    let job = ports
        .journal
        .issue(&action.key, &recheck)
        .map_err(ExecutorError::Journal)?;
    report.issued += 1;
    let outcome = ports.backend.apply(&job, action, staged);
    release(ports, action, staged, &outcome, report)?;
    match &outcome {
        JobOutcome::Unknown => {
            report.unknown += 1;
            Ok(())
        }
        JobOutcome::Committed { .. } | JobOutcome::NoEffect { .. } => {
            ports
                .journal
                .record_outcome(&job, &outcome)
                .map_err(ExecutorError::Journal)?;
            if matches!(outcome, JobOutcome::Committed { .. }) {
                report.committed += 1;
            } else {
                report.no_effect += 1;
            }
            Ok(())
        }
    }
}

fn stage(
    ports: &mut ExecutionPorts<'_>,
    action: &Action,
) -> Result<Option<ObjectId>, ExecutorError> {
    // Canonical 侧的保全与可恢复删除记录归 Broker；这里只管受控镜像侧对象。
    let (side, version) = match (action.key.target, &action.change) {
        (Side::Canonical, _) => return Ok(None),
        (Side::Mirror, Change::CreateOnly { source })
        | (Side::Mirror, Change::Replace { source, .. }) => (Side::Canonical, source),
        (Side::Mirror, Change::Delete { target_expected }) => (Side::Mirror, target_expected),
    };
    transfer::stage(
        ports.cas,
        ports.content,
        ports.sink,
        side,
        &action.key.path,
        version,
    )
    .map(Some)
}

fn release(
    ports: &mut ExecutionPorts<'_>,
    action: &Action,
    staged: Option<ObjectId>,
    outcome: &JobOutcome,
    report: &mut ExecutionReport,
) -> Result<(), ExecutorError> {
    let Some(id) = staged else {
        return Ok(());
    };
    // 删除的保全记录必须留下；复制用的暂存对象在确认安装后交还额度。
    let preserve = matches!(action.change, Change::Delete { .. });
    let keep = match outcome {
        JobOutcome::Committed { .. } | JobOutcome::Unknown => preserve,
        JobOutcome::NoEffect { .. } => false,
    };
    ports
        .cas
        .release_prepared(keep)
        .map_err(ExecutorError::Cas)?;
    if keep {
        report.preserved.push(id);
        return Ok(());
    }
    if matches!(outcome, JobOutcome::Unknown) {
        // 终态未知：保全记录留作孤儿等原键核对，不声称它已生效也不回收额度。
        return Ok(());
    }
    ports.cas.forget(id).map_err(ExecutorError::Cas)
}
