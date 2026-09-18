//! 只读状态投影：四项分开报告，绝不把其中任何一项渲染成「同步成功」。

use serde::Serialize;
use studiolo_sync_transport::owned_run::{ExitObservation, Quiescence, RunObservation, RunStatus};

/// 一次运行的对外视图。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TransportRunView {
    /// 运行标识的十六进制形式。
    pub run: String,
    /// 运行序号。
    pub sequence: u64,
    /// 阶段名。
    pub phase: String,
    /// 静止是否已被证明。
    pub quiescence_confirmed: bool,
    /// 是否实际取得进程等待结果。
    pub exit_observed: bool,
    /// 实际退出码；未观察到时为 `None`。
    pub exit_code: Option<i32>,
    /// 实际终止信号；未观察到时为 `None`。
    pub exit_signal: Option<i32>,
    /// 已持久的停止原因。
    pub stop_reason: Option<String>,
}

/// 传输槽的对外视图。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TransportStatusView {
    /// 持久提交代次。
    pub generation: u64,
    /// 下一个待分配序号。
    pub next_sequence: u64,
    /// 是否还有待补审计事件。
    pub audit_pending: bool,
    /// 至多一个活动运行。
    pub active: Option<TransportRunView>,
    /// 至多一个已审计闭合摘要。
    pub last_terminal: Option<TransportRunView>,
}

/// 把安全逻辑元数据投影成对外视图；不含 stdout、token 或原配置。
#[must_use]
pub fn status_view(status: &RunStatus) -> TransportStatusView {
    TransportStatusView {
        generation: status.generation,
        next_sequence: status.next_sequence,
        audit_pending: status.audit_pending,
        active: status.active.as_ref().map(|active| {
            run_view(
                &format!("{}", active.run_id),
                active.sequence,
                &active.observation,
            )
        }),
        last_terminal: status.last_terminal.as_ref().map(|terminal| {
            run_view(
                &format!("{}", terminal.run_id),
                terminal.sequence,
                &terminal.observation,
            )
        }),
    }
}

fn run_view(run: &str, sequence: u64, observation: &RunObservation) -> TransportRunView {
    TransportRunView {
        run: run.to_owned(),
        sequence,
        phase: format!("{:?}", observation.phase),
        quiescence_confirmed: observation.quiescence == Quiescence::Confirmed,
        exit_observed: observation.exit.reaped(),
        exit_code: match observation.exit {
            ExitObservation::Code(code) => Some(code),
            _ => None,
        },
        exit_signal: match observation.exit {
            ExitObservation::Signal(signal) => Some(signal),
            _ => None,
        },
        stop_reason: observation.stop_reason.map(|reason| format!("{reason:?}")),
    }
}
