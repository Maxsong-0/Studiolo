//! 第 10 步续做：上一批仍有 Issued 未知终态的动作时，按**原键**向后端核对权威终态并写回执。
//!
//! 只核对、不重放：canonical 侧靠 Broker 的按键重放（同一幂等键再提交只返回原回执，绝不第二次安装）；
//! 镜像侧靠静止窗口内的目标版本核对（目标已是冻结源版本＝已提交，仍是冻结前置版本＝未安装）。
//! 两者都核不出的保持未知，交给健康页降级；绝不用「内容碰巧相同」以外的推断补成功。
//! 编排与判定只依赖 [`ReconcilePort`]；真实端口的两侧读法在 `reconcile/{canonical,mirror}.rs`。

mod canonical;
mod mirror;

use super::content::RoundContext;
use super::evidence::hex;
use super::execute::{bridge_error, executor_error};
use crate::sync_plane::PlaneError;
use crate::sync_plane::job_backend::{SeamVerdict, not_attempted, rejected};
use crate::sync_plane::quiescence;
use studiolo_audit_ledger::AuditLedger;
use studiolo_sync_bridge::executor::{
    ExecutorError, IssuedJob, JobOutcome, NoEffectCause, PlanJournal,
};
use studiolo_sync_bridge::sqlite::{ActionStatus, BridgeState, StateJournal};
use studiolo_sync_bridge::{Action, Change, Side};

/// 一次核对的结果计数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// 本轮按原键核出权威终态并已写回执的动作数。
    pub resolved: usize,
    /// 核对后仍未知的动作数。
    pub still_unknown: usize,
}

/// 镜像目标在静止窗口内相对冻结版本的现状（逐字节比对，不看修改时间）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorTarget {
    /// 目标不存在（父目录缺失同样算）。
    Missing,
    /// 目标逐字节等于冻结源版本。
    Source,
    /// 目标逐字节等于冻结前置版本。
    Expected,
    /// 既非源也非前置，或读不出（链接、特殊文件、视图漂移）。
    Other,
}

/// 核对一条未知动作所需的后端能力；真实实现经接缝与视图，契约测试用假实现。
pub trait ReconcilePort {
    /// canonical 侧：按原键重建首次请求走接缝重放；还原不出原前置即 `Unknown`。
    fn replay_canonical(&mut self, job: &IssuedJob, action: &Action) -> SeamVerdict;
    /// 写者此刻能否接上；`NotAttempted` 之后复核，用来区分「写者缺席」与「Broker 对同一请求确定拒收」。
    fn writer_attached(&mut self) -> bool;
    /// 镜像侧：静止窗口内目标相对冻结源／前置版本的现状。
    fn mirror_target(&mut self, action: &Action) -> MirrorTarget;
    /// 读完镜像之后静止是否仍成立；不成立则本轮镜像侧结论一律作废。
    fn still_quiescent(&mut self) -> bool;
}

/// 核对上一批全部 Issued 未知动作。
///
/// # Errors
/// 持久层或后端不可用时返回对应分类；任何一条核不出都不是错误，只计入 `still_unknown`。
pub fn reconcile_pending(context: &RoundContext<'_>) -> Result<ReconcileReport, PlaneError> {
    let plane = context.plane;
    // 第 7 步的静止票据就在上下文里：读镜像前复验一次，读完再复验一次（`still_quiescent`）。
    quiescence::verify(plane, context.ticket)?;
    let mut port = RealPort { context };
    let mut bridge = plane.bridge().lock().map_err(|_| PlaneError::Unavailable)?;
    reconcile_with(&mut port, &mut bridge, plane.audit().as_ref())
}

/// 编排：只取仍是 `OutcomeUnknown` 的原键，逐条问端口，核出的经执行器端口 `StateJournal` 写回执。
///
/// # Errors
/// 持久层读失败按 `bridge_error` 分类；写回执失败按 `executor_error` 同款映射。
pub fn reconcile_with(
    port: &mut dyn ReconcilePort,
    bridge: &mut BridgeState,
    ledger: &dyn AuditLedger,
) -> Result<ReconcileReport, PlaneError> {
    let Some(frozen) = bridge.frozen_pending().map_err(bridge_error)? else {
        return Ok(ReconcileReport::default());
    };
    let mut verdicts = Vec::new();
    let mut mirror_seen = false;
    for view in bridge.retry_pending(&frozen).map_err(bridge_error)? {
        if view.status != ActionStatus::OutcomeUnknown {
            continue;
        }
        let pending = bridge
            .issued_request(&view.action.key)
            .map_err(bridge_error)?;
        let job = IssuedJob {
            key: pending.action.key.clone(),
            external_key: pending.external_key,
            request_digest: pending.request_digest,
        };
        let action = pending.action;
        let outcome = match action.key.target {
            Side::Canonical => {
                let verdict = port.replay_canonical(&job, &action);
                let attached = verdict == SeamVerdict::NotAttempted && port.writer_attached();
                replay_outcome(verdict, attached, &job.external_key)
            }
            Side::Mirror => {
                mirror_seen = true;
                mirror_verdict(&action, port.mirror_target(&action), &job.external_key)
            }
        };
        verdicts.push((job, action.key.target, outcome));
    }
    let quiescent = !mirror_seen || port.still_quiescent();
    let mut report = ReconcileReport::default();
    let mut journal = StateJournal::new(bridge, ledger, frozen);
    for (job, side, outcome) in verdicts {
        if outcome == JobOutcome::Unknown || (side == Side::Mirror && !quiescent) {
            report.still_unknown += 1;
            continue;
        }
        journal
            .record_outcome(&job, &outcome)
            .map_err(|fault| executor_error(ExecutorError::Journal(fault)))?;
        report.resolved += 1;
    }
    Ok(report)
}

/// 重开核对的 canonical 映射：原回执照收；CAS 拒绝记无效果；`NotAttempted` 只有写者此刻接得上
/// （Broker 对同一请求确定拒收：受保护路径／预算超限／原键已明确取消）才是无效果——写者缺席时
/// 这次查询什么也没证明，保持未知。写者缺席是单调的（关停、根漂移都不会自行恢复）。
#[must_use]
pub fn replay_outcome(
    verdict: SeamVerdict,
    writer_attached: bool,
    external_key: &str,
) -> JobOutcome {
    match verdict {
        SeamVerdict::Outcome(outcome) => outcome,
        SeamVerdict::Rejected => rejected(external_key),
        SeamVerdict::NotAttempted if writer_attached => not_attempted(external_key),
        SeamVerdict::NotAttempted | SeamVerdict::Unknown => JobOutcome::Unknown,
    }
}

/// 镜像侧判定：目标已是冻结源 → 已提交；Delete 且目标已不存在 → 已提交；目标仍是冻结前置
/// （CreateOnly 的前置就是「不存在」）→ 未安装；其余一律保持未知。
#[must_use]
pub fn mirror_verdict(action: &Action, target: MirrorTarget, external_key: &str) -> JobOutcome {
    match (&action.change, target) {
        (Change::CreateOnly { source } | Change::Replace { source, .. }, MirrorTarget::Source) => {
            JobOutcome::Committed {
                new_revision: Some(source.sha256),
                content_hash: Some(source.sha256),
                bytes: Some(source.bytes),
                receipt_id: format!("mirror:{}", hex(&source.sha256)),
            }
        }
        (Change::Delete { .. }, MirrorTarget::Missing) => JobOutcome::Committed {
            new_revision: None,
            content_hash: None,
            bytes: None,
            receipt_id: format!("mirror-deleted:{external_key}"),
        },
        (Change::Replace { .. } | Change::Delete { .. }, MirrorTarget::Expected)
        | (Change::CreateOnly { .. }, MirrorTarget::Missing) => JobOutcome::NoEffect {
            reason: NoEffectCause::CancelledBeforeInstall,
            terminal_id: format!("not-installed:{external_key}"),
        },
        _ => JobOutcome::Unknown,
    }
}

/// 真实端口：借本轮上下文（含第 7 步的静止票据）；canonical 只经接缝，镜像只经视图读。
struct RealPort<'r, 'c> {
    context: &'r RoundContext<'c>,
}

impl ReconcilePort for RealPort<'_, '_> {
    fn replay_canonical(&mut self, job: &IssuedJob, action: &Action) -> SeamVerdict {
        canonical::replay(self.context, job, action)
    }

    fn writer_attached(&mut self) -> bool {
        self.context.plane.workbench().bridge_flush_audit().is_ok()
    }

    fn mirror_target(&mut self, action: &Action) -> MirrorTarget {
        mirror::target(self.context, action)
    }

    fn still_quiescent(&mut self) -> bool {
        quiescence::verify(self.context.plane, self.context.ticket).is_ok()
    }
}
