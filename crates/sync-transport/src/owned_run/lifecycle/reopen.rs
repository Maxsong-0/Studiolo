//! 重开路径：只观察旧容器、只在完整匹配时停止、跨 boot 收束；绝不重新执行。
//! 从 `lifecycle.rs` 拆出只为压住单文件行数；阶段边界与那边完全一致。

use super::observation_of;
use crate::linux::{ContainmentBackend, IdentityMatch};
use crate::owned_run::error::{Result, TransportError};
use crate::owned_run::machine::{self, Transition};
use crate::owned_run::session::RunSession;
use crate::owned_run::state::{QuiescenceEvidence, QuiescenceProof, TransportState};
use crate::owned_run::types::{RunId, RunObservation, StopReason};

impl<B: ContainmentBackend> RunSession<'_, B> {
    /// 只观察旧容器，绝不重新执行。
    pub(crate) fn recover(&mut self, run_id: RunId) -> Result<RunObservation> {
        let state = self.load()?;
        let record = state.active.as_ref().ok_or(TransportError::NotFound)?;
        if record.run_id != run_id {
            return Err(TransportError::NotFound);
        }
        if !record.phase.execution_may_have_started() {
            return Ok(record.observation());
        }
        let current_boot = self
            .backend
            .boot_id()
            .map_err(crate::linux::ContainmentGap::after_start)?;
        match record.container.clone() {
            // 尚未固定容器身份：只有跨 boot 才能收束，且原退出结果永远未知。
            None => {
                if record
                    .requested_boot_id
                    .is_some_and(|old| old != current_boot)
                {
                    self.close_by_boot_change(&state)?;
                }
            }
            Some(container) => {
                let matched = self
                    .backend
                    .match_identity(run_id, &container)
                    .map_err(crate::linux::ContainmentGap::after_start)?;
                match matched {
                    IdentityMatch::Exact => return self.observe(run_id),
                    IdentityMatch::BootChanged if current_boot != container.boot_id => {
                        self.close_by_boot_change(&state)?;
                    }
                    // 缺 unit、换 unit、cgroup ABA、同 boot 的「boot 变化」自报都不是证明。
                    _ => {}
                }
            }
        }
        observation_of(&self.load()?, run_id)
    }

    /// 只在完整匹配旧容器时停止；显式请求，绝不进程名搜索。
    pub(crate) fn stop_recovered(&mut self, run_id: RunId) -> Result<()> {
        let state = self.load()?;
        let record = state.active.as_ref().ok_or(TransportError::NotFound)?;
        if record.run_id != run_id {
            return Err(TransportError::NotFound);
        }
        let container = record
            .container
            .clone()
            .ok_or(TransportError::ExitUnconfirmed)?;
        if self
            .backend
            .match_identity(run_id, &container)
            .map_err(crate::linux::ContainmentGap::after_start)?
            != IdentityMatch::Exact
        {
            return Err(TransportError::ExitUnconfirmed);
        }
        let next = machine::apply(&state, Transition::RequestStop(StopReason::OwnerRequested))?;
        self.commit(&next)?;
        self.enforce_stop(&next)
    }

    fn close_by_boot_change(&mut self, state: &TransportState) -> Result<()> {
        let next = machine::apply(
            state,
            Transition::Quiesce(QuiescenceProof {
                evidence: QuiescenceEvidence::BootChanged,
                cgroup_identity: None,
                spawn_gate_closed: true,
            }),
        )?;
        self.commit(&next)?;
        let state = self.load()?;
        let next = machine::apply(&state, Transition::FixOutcome)?;
        self.commit(&next)
    }
}
