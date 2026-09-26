//! 运行生命周期：阶段持久边界严格照契约，观察结果四项分开报告。
//!
//! 启动请求发出**之前**的失败是「不支持受控隔离」；发出**之后**的失败一律保持未知，
//! 保留原槽，不取 token、不启新运行，更不会重新执行。重开路径（`recover`／`stop_recovered`）
//! 在 `lifecycle/reopen.rs`。

mod reopen;

use super::error::{Result, TransportError};
use super::machine::{self, Transition};
use super::session::{RunSession, random};
use super::state::{QuiescenceEvidence, QuiescenceProof, TransportState};
use super::tickets::{OwnedRun, PreparedRun};
use super::types::{
    ContainerIdentity, ExitObservation, ResourceCounts, RunId, RunObservation, RunPhase, StopReason,
};
use crate::linux::launcher::{HandshakeNonce, LaunchRequest, isolation_sufficient};
use crate::linux::{ContainmentBackend, cgroup, service};

impl<B: ContainmentBackend> RunSession<'_, B> {
    /// Intent ACK 与最终绑定复验后才放行；借用票据至多放行一次。
    pub(crate) fn start(&mut self, prepared: &PreparedRun) -> Result<OwnedRun> {
        let state = self.load()?;
        self.check_ticket(&prepared.binding, &state)?;
        let record = state.active.as_ref().ok_or(TransportError::NotFound)?;
        if self.released == Some(record.run_id) || record.phase != RunPhase::Prepared {
            return Err(if record.phase.execution_may_have_started() {
                TransportError::AlreadyStarted
            } else {
                TransportError::PhaseConflict
            });
        }
        if record.intent.pending() {
            return Err(TransportError::AuditPending);
        }
        let (run_id, sequence, operation, budget) = (
            record.run_id,
            record.sequence,
            record.operation,
            record.budget,
        );
        let boot_id = self
            .backend
            .boot_id()
            .map_err(crate::linux::ContainmentGap::before_start)?;
        let argv = service::build_argv(self.runtime.pins(), operation, &state.roots.mirror_root)
            .map_err(crate::linux::ContainmentGap::before_start)?;
        // ① LaunchIssued 必须在请求启动 launcher 之前可靠发布。
        let next = machine::apply(&state, Transition::IssueLaunch { boot_id })?;
        self.commit(&next)?;
        // 从这一刻起本实例不再受理同一票据；重复请求不得产生第二次 spawn。
        self.released = Some(run_id);

        let mut nonce = [0u8; 32];
        random(&mut nonce)?;
        let request = LaunchRequest {
            run_id,
            sequence,
            operation,
            payload_uid: next.backend.payload_uid,
            payload_gid: next.backend.payload_gid,
            budget,
            argv,
            nonce: HandshakeNonce::from_bytes(nonce),
        };
        let ack = self
            .backend
            .issue_launch(&request)
            .map_err(crate::linux::ContainmentGap::after_start)?;
        container_acceptable(&next, &ack.container, boot_id)?;
        isolation_sufficient(&ack.isolation).map_err(crate::linux::ContainmentGap::after_start)?;
        cgroup::satisfies(&ack.limits, &budget)
            .map_err(crate::linux::ContainmentGap::after_start)?;
        if ack.release.run_id() != run_id
            || ack.release.launcher_session() != ack.container.launcher_session
        {
            return Err(TransportError::ExitUnconfirmed);
        }
        // ② 固定身份并核对隔离/预算之后才记 Contained；此时尚未放行 payload。
        let state = self.load()?;
        let next = machine::apply(
            &state,
            Transition::Contain {
                container: Box::new(ack.container.clone()),
                diagnostics: ack.diagnostics,
            },
        )?;
        self.commit(&next)?;
        // ③ ExecutionIssued 必须在放行屏障之前可靠发布。
        let state = self.load()?;
        let next = machine::apply(&state, Transition::IssueExecution)?;
        self.commit(&next)?;
        // ④ 一次性放行；通信丢失不得重发产生第二次执行。
        self.backend
            .release_once(ack.release)
            .map_err(crate::linux::ContainmentGap::after_start)?;
        Ok(OwnedRun {
            binding: self.ticket(&next, run_id, sequence)?,
        })
    }

    /// 停止原因先持久，再只对绑定子组施加有界 grace 的停止。
    pub(crate) fn request_stop(&mut self, owned: &OwnedRun, reason: StopReason) -> Result<()> {
        let state = self.load()?;
        self.check_ticket(&owned.binding, &state)?;
        let next = machine::apply(&state, Transition::RequestStop(reason))?;
        self.commit(&next)?;
        self.enforce_stop(&next)
    }

    /// 推进到当前可证明的观察；不自旋、不睡眠，也不把「没证据」当成好结果。
    pub(crate) fn wait(&mut self, owned: &OwnedRun) -> Result<RunObservation> {
        let state = self.load()?;
        self.check_ticket(&owned.binding, &state)?;
        self.observe(owned.binding.run_id)
    }

    fn enforce_stop(&mut self, state: &TransportState) -> Result<()> {
        let Some(record) = state.active.as_ref() else {
            return Ok(());
        };
        let Some(container) = record.container.clone() else {
            // 还没有容器身份就没有可停止的对象；停止事实已持久，等待恢复流程。
            return Ok(());
        };
        self.backend
            .stop(record.run_id, &container, record.budget.stop_grace_seconds)
            .map_err(crate::linux::ContainmentGap::after_start)
    }

    fn observe(&mut self, expect: RunId) -> Result<RunObservation> {
        let mut state = self.load()?;
        loop {
            let record = state.active.as_ref().ok_or(TransportError::NotFound)?;
            if record.run_id != expect {
                return Err(TransportError::NotFound);
            }
            let Some(transition) = self.next_step(&state)? else {
                break;
            };
            let stopping = matches!(transition, Transition::RequestStop(_));
            state = machine::apply(&state, transition)?;
            self.commit(&state)?;
            if stopping {
                self.enforce_stop(&state)?;
            }
            state = self.load()?;
        }
        observation_of(&state, expect)
    }

    fn next_step(&mut self, state: &TransportState) -> Result<Option<Transition>> {
        let record = state.active.as_ref().ok_or(TransportError::NotFound)?;
        if record.phase == RunPhase::Quiesced {
            return Ok(Some(Transition::FixOutcome));
        }
        let Some(container) = record.container.clone() else {
            return Ok(None);
        };
        if !matches!(
            record.phase,
            RunPhase::ExecutionIssued | RunPhase::Running | RunPhase::StopRequested
        ) {
            return Ok(None);
        }
        let run_id = record.run_id;
        let counts = self
            .backend
            .drain(run_id, &container)
            .map_err(crate::linux::ContainmentGap::after_start)?;
        if grew(&record.counts, &counts) {
            return Ok(Some(Transition::RecordCounts(counts)));
        }
        if record.counts.output_over_budget && record.stop_reason.is_none() {
            return Ok(Some(Transition::RequestStop(
                StopReason::OutputBudgetExceeded,
            )));
        }
        if record.phase == RunPhase::ExecutionIssued
            && self
                .backend
                .observe_running(run_id, &container)
                .map_err(crate::linux::ContainmentGap::after_start)?
        {
            return Ok(Some(Transition::ObserveRunning));
        }
        // 实际等待结果与静止证明分开取；两者互不替代。
        let exit = self
            .backend
            .reap(run_id, &container)
            .map_err(crate::linux::ContainmentGap::after_start)?;
        if exit != ExitObservation::NotObserved && exit != record.exit {
            return Ok(Some(Transition::RecordExit(exit)));
        }
        self.backend
            .close_spawn_gate(run_id, &container)
            .map_err(crate::linux::ContainmentGap::after_start)?;
        let events = self
            .backend
            .read_events(run_id, &container)
            .map_err(crate::linux::ContainmentGap::after_start)?;
        if events.populated {
            return Ok(None);
        }
        Ok(Some(Transition::Quiesce(QuiescenceProof {
            evidence: QuiescenceEvidence::PayloadCgroupUnpopulated,
            cgroup_identity: Some(container.cgroup_identity),
            spawn_gate_closed: true,
        })))
    }
}

fn grew(current: &ResourceCounts, next: &ResourceCounts) -> bool {
    next.stdout_bytes > current.stdout_bytes
        || next.stderr_bytes > current.stderr_bytes
        || (next.output_over_budget && !current.output_over_budget)
}

fn observation_of(state: &TransportState, run_id: RunId) -> Result<RunObservation> {
    if let Some(record) = state.active.as_ref() {
        if record.run_id != run_id {
            return Err(TransportError::NotFound);
        }
        return Ok(record.observation());
    }
    state
        .last_terminal
        .as_ref()
        .filter(|terminal| terminal.run_id == run_id)
        .map(|terminal| terminal.observation)
        .ok_or(TransportError::NotFound)
}

fn container_acceptable(
    state: &TransportState,
    container: &ContainerIdentity,
    boot_id: [u8; 16],
) -> Result<()> {
    let acceptable = container.boot_id == boot_id
        && container.unit_name == state.backend.service_unit
        && service::unit_name_acceptable(&container.unit_name)
        && cgroup::within_prefix(
            &container.payload_cgroup,
            &state.backend.payload_cgroup_prefix,
        )
        && container.cgroup_identity != (0, 0)
        && container.invocation_id != [0u8; 16]
        && container.launcher_session != [0u8; 16];
    if acceptable {
        Ok(())
    } else {
        Err(TransportError::ExitUnconfirmed)
    }
}
