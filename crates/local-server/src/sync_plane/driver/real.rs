//! `RoundPorts` 的真实实现：持久小状态、外 stage、传输运行槽、Bridge 持久状态与接缝的装配。
//!
//! 每轮新开一份：`plane-state.json` 与外 stage 在开工时打开（失败即 `Storage`），
//! 静止票据在第 7 步取得后供观察、核对、冲突保全与执行复用。

use super::super::health::now_unix_seconds;
use super::super::mirror::MirrorView;
use super::super::mirror::stage::StageStore;
use super::super::recovery::{self, PlaneState, PlaneStateStore};
use super::super::{PlaneError, SyncPlane, quiescence};
use super::content::RoundContext;
use super::observe::Observed;
use super::{
    HeadlessRun, Inspection, RoundOutcome, RoundPorts, approved_mode, execute, headless, observe,
    preserve, reconcile, resolve,
};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use studiolo_sync_bridge::sqlite::ActionStatus;
use studiolo_sync_bridge::{Decision, Input, Inventory};
use studiolo_sync_transport::owned_run::{QuiescentMirror, RunOperation, SyncMode};

/// 一轮的真实端口。
pub struct RealPorts<'a> {
    plane: &'a SyncPlane,
    now_unix: i64,
    store: PlaneStateStore,
    stage: RefCell<StageStore>,
    /// 本轮应处的模式；`inspect` 时记下，供 headless 回填判断是否为强制恢复运行。
    mode: SyncMode,
    /// 审批文档里的模式；与 `mode` 不同即本轮在跑 D9 的强制 `mirror-remote` 恢复。
    approved: SyncMode,
    /// `drive_until` 的关停标志：安全停点据此收尾，headless 轮询据此 `request_stop`。
    stopping: &'a AtomicBool,
    ticket: Option<QuiescentMirror>,
    observed: Option<Observed>,
}

impl<'a> RealPorts<'a> {
    /// 打开本轮需要的私有存储；任一失败即本轮不跑。
    ///
    /// # Errors
    /// `plane-state.json` 或外 stage 不可用返回 `Storage`。
    pub fn open(
        plane: &'a SyncPlane,
        now_unix: i64,
        stopping: &'a AtomicBool,
    ) -> Result<Self, PlaneError> {
        Ok(Self {
            plane,
            now_unix,
            store: PlaneStateStore::open(plane.plane_root())?,
            stage: RefCell::new(StageStore::open(plane.plane_root())?),
            mode: SyncMode::MirrorRemote,
            approved: approved_mode(plane.approval().headless_mode()),
            stopping,
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
            ticket,
            view: MirrorView::new(ticket.mirror_root().to_path_buf(), ticket.mirror_identity()),
            stage: &self.stage,
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
        let run = self.run_headless(RunOperation::InspectConfig)?;
        if !run.clean {
            return Err(run.failure());
        }
        let output = self
            .plane
            .transport()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .config_output(run.run_id)
            .map_err(|_| PlaneError::Transport)?;
        let plane = self.plane;
        let inspection = headless::classify(
            expected,
            plane.approval().excluded_folders(),
            &self.store.state().verified_excluded,
            &mut |excluded, mode| {
                headless::verify_config(plane, output.as_bytes(), mode, excluded).is_ok()
            },
        )?;
        if inspection.mode_matches {
            plane.health.record_mode(expected);
        }
        Ok(inspection)
    }

    fn residue_blocked(
        &mut self,
        previous: &[String],
        current: &[String],
    ) -> Result<bool, PlaneError> {
        // 守卫要读镜像视图：先取 InspectConfig 刚闭合留下的静止票据，读完复验，作废即 `Quiescence`。
        let ticket = self
            .plane
            .transport()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .quiescent()
            .map_err(|_| PlaneError::Quiescence)?;
        let residue = recovery::residue_guard(previous, current, ticket.mirror_root())?;
        quiescence::verify(self.plane, &ticket)?;
        Ok(residue.is_some())
    }

    fn set_mode(&mut self, mode: SyncMode) -> Result<HeadlessRun, PlaneError> {
        self.run_headless(RunOperation::SetMode(mode))
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
        self.run_headless(RunOperation::RunOnce)
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

    fn preserve_conflicts(&mut self, decision: &Decision) -> Result<u64, PlaneError> {
        // 戳先落盘、后写副本：写到一半失败或进程中断，下一轮沿用同一戳、同一路径与幂等键，不另生一份。
        let link_id = self.context()?.binding.link_id;
        let mut state = self.store.state().clone();
        let stamps = &mut state.conflict_stamps;
        let added = preserve::assign_stamps(&link_id, decision, stamps, self.now_unix)?;
        if state != *self.store.state() {
            self.store.save(state)?;
        }
        // 冲突一经配戳落盘即计入健康节（副本由本轮或后续轮按同一戳幂等写出）。
        self.plane.health.record_conflicts(added);
        let device = self.plane.approval().device_name();
        let stamps = &self.store.state().conflict_stamps;
        preserve::preserve_conflicts(&self.context()?, decision, stamps, device)?;
        Ok(added)
    }

    fn execute(&mut self, input: Input) -> Result<RoundOutcome, PlaneError> {
        let observed = self.observed.take().ok_or(PlaneError::Observation)?;
        if observed.input != input {
            return Err(PlaneError::Observation);
        }
        let outcome = execute::reserve_and_execute(&self.context()?, &observed);
        // 执行段回填（W5 `record_execution`）：序号、终态未知数、熔断是否生效；冲突副本数由健康侧合计。
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
            let health = &self.plane.health;
            health.record_execution(sequence, unknown, fused);
        }
        outcome
    }

    fn stopping(&mut self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }
}

impl RealPorts<'_> {
    /// 跑一次 headless 并回填健康；本轮应处模式与审批模式不同即强制恢复运行。
    fn run_headless(&self, operation: RunOperation) -> Result<HeadlessRun, PlaneError> {
        let run = headless::run(self.plane, operation, self.now_unix, self.stopping)?;
        let forced =
            self.mode == SyncMode::MirrorRemote && self.approved == SyncMode::Bidirectional;
        self.plane.health.record_headless(&run, forced);
        Ok(run)
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
