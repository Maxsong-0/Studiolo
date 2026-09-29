//! `RoundPorts` 的真实实现：持久小状态、外 stage、传输运行槽、Bridge 持久状态与接缝的装配。
//!
//! 每轮新开一份：`plane-state.json` 与外 stage 在开工时打开（失败即 `Storage`），
//! 静止票据在第 7 步取得后供观察、核对、冲突保全与执行复用。
mod ports;

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
        let store = PlaneStateStore::open(plane.plane_root())?;
        plane
            .health
            .record_conflicts(store.state().conflict_stamps.len() as u64);
        Ok(Self {
            plane,
            now_unix,
            store,
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
