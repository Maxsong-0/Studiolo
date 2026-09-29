//! 假端口：按驱动器的调用逐条记录，并按脚本返回固定结论；不碰 headless、Broker 或磁盘。

use super::fixture;
use studiolo_local_server::sync_plane::PlaneError;
use studiolo_local_server::sync_plane::driver::{
    HeadlessRun, Inspection, RoundOutcome, RoundPorts, headless,
};
use studiolo_local_server::sync_plane::recovery::PlaneState;
use studiolo_sync_bridge::{Decision, Input, Inventory};
use studiolo_sync_transport::owned_run::{StopReason, SyncMode};

/// 驱动器对端口的一次调用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    State,
    Inspect(SyncMode),
    Residue,
    SetMode(SyncMode),
    NextSequence,
    RunOnce,
    Save,
    Quiescent,
    Unresolved,
    Observe(SyncMode),
    Preresolve,
    Preserve,
    Execute,
}

/// 假端口的脚本与记录。
pub struct FakePorts {
    /// 当前持久小状态；`save_state` 会替换它并追加到 `saved`。
    pub state: PlaneState,
    /// headless 当前所处模式；`set_mode` 会改它。
    pub current_mode: SyncMode,
    /// 文件类型等其余配置是否与审批一致；不一致时 `inspect` 直接报 `ConfigMismatch`。
    pub config_consistent: bool,
    /// 审批里的排除文件夹。
    pub excluded: Vec<String>,
    /// headless 本地实际配置的排除文件夹；`inspect` 按真实端口同一核对顺序算 `pending_purge`。
    pub headless_excluded: Vec<String>,
    /// 排除项守卫是否发现残留。
    pub residue: bool,
    /// RunOnce 的退出码。
    pub run_once_exit: i32,
    /// 是否能拿到静止证据。
    pub quiescent: bool,
    /// 上一批仍有未知终态的计划序号。
    pub unresolved: Option<u64>,
    /// `observe` 返回的冻结输入。
    pub input: Input,
    /// `preserve_conflicts` 的结论。
    pub preserve_result: Result<u64, PlaneError>,
    /// `execute` 的结论；返回 `Unknown` 时假端口会把它记成 `unresolved`。
    pub execute_result: Result<RoundOutcome, PlaneError>,
    /// 调用数达到这个值起关停标志为真；RunOnce 恰在其中时模拟被 `request_stop(OwnerRequested)` 停下。
    pub stop_after: Option<usize>,
    /// 下一个运行序号。
    pub next_sequence: u64,
    /// 调用序列。
    pub calls: Vec<Call>,
    /// 每次保存的持久小状态快照。
    pub saved: Vec<PlaneState>,
}

impl FakePorts {
    /// 一切正常的脚本：模式一致、无残留、RunOnce 干净、静止可得、执行闭合；
    /// 上一轮已记下 headless 现状（与审批同一排除集合），本轮不因它多存一次。
    pub fn healthy(current_mode: SyncMode) -> Self {
        let excluded = vec!["private".to_owned()];
        let mut state = fixture::first_run_state();
        state.headless_excluded = excluded.clone();
        Self {
            state,
            current_mode,
            config_consistent: true,
            headless_excluded: excluded.clone(),
            excluded,
            residue: false,
            run_once_exit: 0,
            quiescent: true,
            unresolved: None,
            input: fixture::empty_input(),
            preserve_result: Ok(0),
            execute_result: Ok(RoundOutcome::Settled {
                sequence: 1,
                baseline_advanced: true,
            }),
            stop_after: None,
            next_sequence: 7,
            calls: Vec::new(),
            saved: Vec::new(),
        }
    }

    /// 某类调用出现的次数。
    pub fn count(&self, wanted: &Call) -> usize {
        self.calls.iter().filter(|call| *call == wanted).count()
    }
}

impl RoundPorts for FakePorts {
    fn state(&mut self) -> Result<PlaneState, PlaneError> {
        self.calls.push(Call::State);
        Ok(self.state.clone())
    }

    fn inspect(&mut self, expected: SyncMode) -> Result<Inspection, PlaneError> {
        self.calls.push(Call::Inspect(expected));
        if !self.config_consistent {
            return Err(PlaneError::ConfigMismatch);
        }
        let current = self.current_mode;
        let mut reported = self.headless_excluded.clone();
        reported.sort();
        reported.dedup();
        let verified = self.state.verified_excluded.clone();
        headless::classify(
            expected,
            &self.excluded,
            &verified,
            &mut |excluded, mode| mode == current && excluded == reported.as_slice(),
        )
    }

    fn residue_blocked(&mut self, _: &[String], _: &[String]) -> Result<bool, PlaneError> {
        self.calls.push(Call::Residue);
        Ok(self.residue)
    }

    fn set_mode(&mut self, mode: SyncMode) -> Result<HeadlessRun, PlaneError> {
        self.calls.push(Call::SetMode(mode));
        self.current_mode = mode;
        Ok(fixture::headless(0))
    }

    fn next_sequence(&mut self) -> Result<u64, PlaneError> {
        self.calls.push(Call::NextSequence);
        Ok(self.next_sequence)
    }

    fn run_once(&mut self) -> Result<HeadlessRun, PlaneError> {
        self.calls.push(Call::RunOnce);
        if self.stopping() {
            // 轮询里见到关停标志：request_stop(OwnerRequested)。headless 对 SIGTERM 以 rc=0 退出
            // （P2 S9a/b 实测），照样闭合回收；分类必须看停止原因而不只看退出码（评审 P2-1）。
            let mut observation = fixture::headless(0).observation;
            observation.stop_reason = Some(StopReason::OwnerRequested);
            return Ok(HeadlessRun::from_terminal(
                fixture::headless(0).run_id,
                observation,
            ));
        }
        Ok(fixture::headless(self.run_once_exit))
    }

    fn save_state(&mut self, state: PlaneState) -> Result<(), PlaneError> {
        self.calls.push(Call::Save);
        self.saved.push(state.clone());
        self.state = state;
        Ok(())
    }

    fn quiescent(&mut self) -> Result<(), PlaneError> {
        self.calls.push(Call::Quiescent);
        if self.quiescent {
            Ok(())
        } else {
            Err(PlaneError::Quiescence)
        }
    }

    fn unresolved(&mut self) -> Result<Option<u64>, PlaneError> {
        self.calls.push(Call::Unresolved);
        Ok(self.unresolved)
    }

    fn observe(&mut self, mode: SyncMode) -> Result<(Input, Inventory, Inventory), PlaneError> {
        self.calls.push(Call::Observe(mode));
        Ok((
            self.input.clone(),
            self.input.canonical.clone(),
            self.input.mirror.clone(),
        ))
    }

    fn preresolve(&mut self, _: &Decision) -> Result<bool, PlaneError> {
        self.calls.push(Call::Preresolve);
        Ok(false)
    }

    fn preserve_conflicts(&mut self, _: &Decision) -> Result<u64, PlaneError> {
        self.calls.push(Call::Preserve);
        self.preserve_result
    }

    fn execute(&mut self, _: Input) -> Result<RoundOutcome, PlaneError> {
        self.calls.push(Call::Execute);
        if let Ok(RoundOutcome::Unknown { sequence }) = self.execute_result {
            self.unresolved = Some(sequence);
        }
        self.execute_result
    }

    fn stopping(&mut self) -> bool {
        self.stop_after
            .is_some_and(|after| self.calls.len() >= after)
    }
}
