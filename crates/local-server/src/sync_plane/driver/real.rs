//! `RoundPorts` 的真实实现：持久小状态、外 stage、传输运行槽、Bridge 持久状态与接缝的装配。
//!
//! 每轮新开一份：`plane-state.json` 与外 stage 在开工时打开（失败即 `Storage`），
//! 静止票据在第 7 步取得后供观察与执行复用，`Revision` 表只活在本轮。

use super::super::mirror::MirrorView;
use super::super::mirror::stage::StageStore;
use super::super::recovery::{self, PlaneState, PlaneStateStore};
use super::super::{PlaneError, SyncPlane};
use super::content::RoundContext;
use super::evidence::RevisionTable;
use super::observe::Observed;
use super::super::health::now_unix_seconds;
use super::{
    HeadlessRun, Inspection, RoundOutcome, RoundPorts, approved_mode, execute, headless, observe,
    reconcile, resolve,
};
use crate::sync_transport_config::ConfigError;
use std::cell::RefCell;
use studiolo_sync_bridge::sqlite::ActionStatus;
use studiolo_sync_bridge::{Decision, Input, Inventory};
use studiolo_sync_transport::owned_run::{QuiescentMirror, RunOperation, SyncMode};

/// 一轮的真实端口。
pub struct RealPorts<'a> {
    plane: &'a SyncPlane,
    now_unix: i64,
    store: PlaneStateStore,
    stage: RefCell<StageStore>,
    revisions: RefCell<RevisionTable>,
    /// 本轮应处的模式；`inspect` 时记下，供 headless 回填判断是否为强制恢复运行。
    mode: SyncMode,
    /// 审批文档里的模式；与 `mode` 不同即本轮在跑 D9 的强制 `mirror-remote` 恢复。
    approved: SyncMode,
    ticket: Option<QuiescentMirror>,
    observed: Option<Observed>,
}

impl<'a> RealPorts<'a> {
    /// 打开本轮需要的私有存储；任一失败即本轮不跑。
    ///
    /// # Errors
    /// `plane-state.json` 或外 stage 不可用返回 `Storage`。
    pub fn open(plane: &'a SyncPlane, now_unix: i64) -> Result<Self, PlaneError> {
        Ok(Self {
            plane,
            now_unix,
            store: PlaneStateStore::open(plane.plane_root())?,
            stage: RefCell::new(StageStore::open(plane.plane_root())?),
            revisions: RefCell::new(RevisionTable::default()),
            mode: SyncMode::MirrorRemote,
            approved: approved_mode(plane.approval().headless_mode()),
            ticket: None,
            observed: None,
        })
    }

    fn context(&self) -> Result<RoundContext<'_>, PlaneError> {
        let ticket = self.ticket.as_ref().ok_or(PlaneError::Quiescence)?;
        let binding = self
            .plane
            .bridge()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .view()
            .map_err(|_| PlaneError::Storage)?
            .cursor
            .binding;
        Ok(RoundContext {
            plane: self.plane,
            view: MirrorView::new(ticket.mirror_root().to_path_buf(), ticket.mirror_identity()),
            stage: &self.stage,
            revisions: &self.revisions,
            binding,
            mode: self.mode,
            now_unix: self.now_unix,
        })
    }
}

impl RoundPorts for RealPorts<'_> {
    fn state(&mut self) -> Result<PlaneState, PlaneError> {
        Ok(self.store.state().clone())
    }

    fn inspect(&mut self, expected: SyncMode) -> Result<Inspection, PlaneError> {
        self.mode = expected;
        let run = headless::run(self.plane, RunOperation::InspectConfig, self.now_unix)?;
        self.record_headless(run);
        if !run.clean {
            return Err(PlaneError::Transport);
        }
        let output = self
            .plane
            .transport()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .config_output(run.run_id)
            .map_err(|_| PlaneError::Transport)?;
        let other = match expected {
            SyncMode::MirrorRemote => SyncMode::Bidirectional,
            SyncMode::Bidirectional => SyncMode::MirrorRemote,
        };
        let mode_matches = match headless::verify_config(self.plane, output.as_bytes(), expected) {
            Ok(()) => true,
            // 只有模式不同：其余各项都和审批一致，交给第 5 步 SetMode。
            Err(ConfigError::Mismatch)
                if headless::verify_config(self.plane, output.as_bytes(), other).is_ok() =>
            {
                false
            }
            Err(_) => return Err(PlaneError::ConfigMismatch),
        };
        if mode_matches {
            self.plane.health.record_mode(expected);
        }
        Ok(Inspection {
            mode_matches,
            excluded_folders: self.plane.approval().excluded_folders().to_vec(),
        })
    }

    fn residue_blocked(
        &mut self,
        previous: &[String],
        current: &[String],
    ) -> Result<bool, PlaneError> {
        let view_root = self.plane.approval().roots().mirror_view_root();
        Ok(recovery::residue_guard(previous, current, view_root)?.is_some())
    }

    fn set_mode(&mut self, mode: SyncMode) -> Result<HeadlessRun, PlaneError> {
        let run = headless::run(self.plane, RunOperation::SetMode(mode), self.now_unix)?;
        self.record_headless(run);
        Ok(run)
    }

    fn next_sequence(&mut self) -> Result<u64, PlaneError> {
        Ok(self
            .plane
            .transport()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .inspect()
            .map_err(headless::transport_error)?
            .next_sequence)
    }

    fn run_once(&mut self) -> Result<HeadlessRun, PlaneError> {
        let run = headless::run(self.plane, RunOperation::RunOnce, self.now_unix)?;
        self.record_headless(run);
        Ok(run)
    }

    fn save_state(&mut self, state: PlaneState) -> Result<(), PlaneError> {
        self.store.save(state)
    }

    fn clock(&mut self) -> Option<i64> {
        Some(now_unix_seconds())
    }

    fn quiescent(&mut self) -> Result<(), PlaneError> {
        let ticket = self
            .plane
            .transport()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .quiescent()
            .map_err(|_| PlaneError::Quiescence)?;
        self.ticket = Some(ticket);
        Ok(())
    }

    fn unresolved(&mut self) -> Result<Option<u64>, PlaneError> {
        // 第 10 步续做：上一批仍有 Issued 未知终态的动作，先按原键向后端核对权威终态并写回执，
        // 核不出的才保持未知（设计文档第 2 节第 10 步「下一轮原键核对」，评审 P1-1）。
        if self.pending_unknown()?.1 > 0 {
            reconcile::reconcile_pending(&self.context()?)?;
        }
        let (sequence, unknown) = self.pending_unknown()?;
        Ok((unknown > 0).then_some(sequence).flatten())
    }

    fn observe(&mut self, mode: SyncMode) -> Result<(Input, Inventory, Inventory), PlaneError> {
        self.mode = mode;
        let observed = observe::observe_both(&self.context()?)?;
        let result = (
            observed.input.clone(),
            observed.canonical.clone(),
            observed.mirror.clone(),
        );
        self.observed = Some(observed);
        Ok(result)
    }

    fn preresolve(&mut self, decision: &Decision) -> Result<bool, PlaneError> {
        let changed = resolve::preresolve_conflicts(&self.context()?, decision)?;
        if changed {
            // 每个 canonical 侧存在的冲突都先保全成一份冲突副本；权威拒绝中途停下的那轮按上限计。
            let preserved = match decision {
                Decision::Ready(plan) => plan.conflicts().len(),
                Decision::FuseBlocked(blocked) => blocked.conflicts.len(),
            };
            self.plane
                .health
                .record_conflicts(u64::try_from(preserved).unwrap_or(u64::MAX));
        }
        Ok(changed)
    }

    fn execute(&mut self, input: Input) -> Result<RoundOutcome, PlaneError> {
        let observed = self.observed.take().ok_or(PlaneError::Observation)?;
        if observed.input != input {
            return Err(PlaneError::Observation);
        }
        let ticket = self.ticket.as_ref().ok_or(PlaneError::Quiescence)?;
        let outcome = execute::reserve_and_execute(&self.context()?, ticket, &observed);
        // 执行段回填（W5 `record_execution`）：序号、终态未知数、冲突副本累计、熔断是否生效。
        if let Ok(
            RoundOutcome::Settled { sequence, .. }
            | RoundOutcome::Unknown { sequence }
            | RoundOutcome::Blocked { sequence },
        ) = outcome
        {
            let (_, unknown) = self.pending_unknown()?;
            let fused = self
                .plane
                .bridge()
                .lock()
                .map_err(|_| PlaneError::Unavailable)?
                .view()
                .map(|view| view.fused)
                .unwrap_or(true);
            self.plane.health.record_execution(
                sequence,
                unknown,
                self.plane.health.conflicts_total(),
                fused,
            );
        }
        outcome
    }
}

impl RealPorts<'_> {
    /// headless 一次运行回收后的健康回填；本轮应处模式与审批模式不同即强制恢复运行。
    fn record_headless(&self, run: HeadlessRun) {
        let forced =
            self.mode == SyncMode::MirrorRemote && self.approved == SyncMode::Bidirectional;
        self.plane
            .health
            .record_headless(run.clean, run.rc_one, forced);
    }

    /// 当前未闭合计划的序号与其中终态未知的动作数；没有未闭合计划即 `(None, 0)`。
    fn pending_unknown(&self) -> Result<(Option<u64>, u64), PlaneError> {
        let bridge = self
            .plane
            .bridge()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?;
        let view = bridge.view().map_err(|_| PlaneError::Storage)?;
        let Some(sequence) = view.pending_sequence else {
            return Ok((None, 0));
        };
        let frozen = bridge
            .frozen_pending()
            .map_err(|_| PlaneError::Storage)?
            .ok_or(PlaneError::Storage)?;
        let unknown = bridge
            .retry_pending(&frozen)
            .map_err(|_| PlaneError::Storage)?
            .iter()
            .filter(|progress| matches!(progress.status, ActionStatus::OutcomeUnknown))
            .count();
        Ok((Some(sequence), u64::try_from(unknown).unwrap_or(u64::MAX)))
    }
}
