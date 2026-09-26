//! 一轮驱动的精确顺序（`docs/m2/sync-plane-design.md` 第 2 节）；每一步失败即停在原地，
//! 下一轮从持久状态续做，绝不补造成功、不重放 Issued。
//!
//! 编排核心 `run_round_with` 只依赖端口 trait [`RoundPorts`]，便于用假端口做契约测试；
//! 真实端口在 `driver/real.rs`，headless 运行、观察、预解决、预留执行各在其子模块。

pub mod content;
pub mod evidence;
pub mod execute;
pub mod headless;
pub mod observe;
pub mod real;
pub mod reconcile;
pub mod resolve;

use super::recovery::{self, PlaneState};
use super::{PlaneError, SyncPlane};
use studiolo_host_config::sync_transport::TransportSyncMode;
use studiolo_sync_bridge::{Decision, Input, Inventory, plan};
use studiolo_sync_transport::owned_run::{RunId, RunObservation, RunOperation, SyncMode};

/// 一轮的结论；每个变体都对应健康页第六节的一种状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundOutcome {
    /// 处于退避窗口，本轮什么都没跑。
    Skipped,
    /// 强制 `mirror-remote` 恢复轮（D9）：上次未干净回收，本轮只跑 SetMode＋RunOnce 并干净回收，
    /// 不观察、不规划、不执行；下一轮才按审批模式跑（评审 P2-1）。
    Recovered,
    /// headless 跑了但没有可搬运的差异（规划结果全部是共同观察）。
    Idle,
    /// 一批已闭合。
    Settled {
        /// 计划序号。
        sequence: u64,
        /// 共同基线是否前进。
        baseline_advanced: bool,
    },
    /// 一批停在未知终态；等下一轮按原键核对。
    Unknown {
        /// 计划序号。
        sequence: u64,
    },
    /// 熔断或冲突冻结了本批（P6 语义下冲突不自动解决）。
    Blocked {
        /// 计划序号。
        sequence: u64,
    },
}

/// headless 一次运行的分类结果；驱动器只看退出码与静止证据，不解析 stderr。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadlessRun {
    /// 本次运行标识；InspectConfig／SetMode 之后凭它取配置输出。
    pub run_id: RunId,
    /// 本次运行的最终观察。
    pub observation: RunObservation,
    /// 退出码是否为 0 且静止已证明。
    pub clean: bool,
    /// 退出码是否恰为 1（残留锁的典型值）。
    pub rc_one: bool,
}

/// InspectConfig 复核的结论；排除项／文件类型不一致不在这里表达，端口直接报 `ConfigMismatch`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
    /// headless 当前 `syncMode` 是否就是本轮应处的模式。
    pub mode_matches: bool,
    /// 本轮核验通过的排除文件夹集合（审批母本，已排序）。
    pub excluded_folders: Vec<String>,
}

/// 一轮驱动用到的全部外部能力；真实实现在 `real.rs`，契约测试用假端口记录调用序列。
pub trait RoundPorts {
    /// 读持久小状态（第 1 步）；撕裂即 `Storage`。
    fn state(&mut self) -> Result<PlaneState, PlaneError>;
    /// 补审计、回收旧槽并跑 InspectConfig 复核（第 2～3 步）。
    fn inspect(&mut self, expected: SyncMode) -> Result<Inspection, PlaneError>;
    /// 排除项守卫（第 4 步）：上次核验集合里有、当前没有的文件夹是否仍有镜像残留。
    fn residue_blocked(
        &mut self,
        previous: &[String],
        current: &[String],
    ) -> Result<bool, PlaneError>;
    /// 跑 SetMode（第 5 步）。
    fn set_mode(&mut self, mode: SyncMode) -> Result<HeadlessRun, PlaneError>;
    /// 下一次运行将得到的序号；持久「已启动」用。
    fn next_sequence(&mut self) -> Result<u64, PlaneError>;
    /// 跑 RunOnce（第 6 步）。
    fn run_once(&mut self) -> Result<HeadlessRun, PlaneError>;
    /// 完整替换持久小状态。
    fn save_state(&mut self, state: PlaneState) -> Result<(), PlaneError>;
    /// 当前墙钟（Unix 秒）；`None` 表示端口不提供时钟，编排沿用轮开始时刻。
    /// 真实端口必须提供：一次 RunOnce 可跑到 `deadline_seconds`，退避与退出时刻要按回收后的钟算（评审 P3-1）。
    fn clock(&mut self) -> Option<i64> {
        None
    }
    /// 取静止证据（第 7 步）；没有即 `Quiescence`。
    fn quiescent(&mut self) -> Result<(), PlaneError>;
    /// 上一批是否仍有未知终态的动作；有则返回其计划序号（第 10 步的续做守卫）。
    fn unresolved(&mut self) -> Result<Option<u64>, PlaneError>;
    /// 两侧完整观察并核准范围，组成冻结输入（第 8 步）。
    fn observe(&mut self, mode: SyncMode) -> Result<(Input, Inventory, Inventory), PlaneError>;
    /// 引导期冲突预解决（第 9 步）；返回是否改动过 canonical。
    fn preresolve(&mut self, decision: &Decision) -> Result<bool, PlaneError>;
    /// 预留、逐动作执行、闭合与保留期回收（第 10～12 步）。
    fn execute(&mut self, input: Input) -> Result<RoundOutcome, PlaneError>;
}

/// 跑完整的一轮；`now_unix` 由调用方注入。
///
/// # Errors
/// 任一步失败返回对应固定分类；调用方只记健康，不重试。
pub fn run_round(plane: &SyncPlane, now_unix: i64) -> Result<RoundOutcome, PlaneError> {
    let approved = approved_mode(plane.approval().headless_mode());
    let mut ports = real::RealPorts::open(plane, now_unix)?;
    run_round_with(&mut ports, approved, now_unix)
}

/// 审批文档里的镜像模式到传输层模式的投影。
#[must_use]
pub fn approved_mode(mode: TransportSyncMode) -> SyncMode {
    match mode {
        TransportSyncMode::MirrorRemote => SyncMode::MirrorRemote,
        TransportSyncMode::Bidirectional => SyncMode::Bidirectional,
    }
}

/// 对端口 trait 的纯编排：设计文档第 2 节十二步逐条对应，停点与续做点见各步注释。
///
/// # Errors
/// 任一步失败返回对应固定分类。
pub fn run_round_with(
    ports: &mut dyn RoundPorts,
    approved: SyncMode,
    now_unix: i64,
) -> Result<RoundOutcome, PlaneError> {
    // 1 持久小状态与恢复判定：退避窗口内什么都不跑。
    let mut state = ports.state()?;
    let decision = recovery::decide(&state, approved, now_unix);
    if decision.backoff {
        return Ok(RoundOutcome::Skipped);
    }
    let mode = decision.effective_mode;
    // 2＋3 补审计、回收旧槽、InspectConfig 复核；排除项与五类不一致由端口直接报 ConfigMismatch。
    let mut inspection = ports.inspect(mode)?;
    // 4 排除项守卫：残留未清就停，不跑 RunOnce。
    if ports.residue_blocked(&state.verified_excluded, &inspection.excluded_folders)? {
        return Err(PlaneError::ResidueBlocked);
    }
    // 5 模式不符才 SetMode，然后回到 3 复核；复核通过后才记 applied_mode。
    if !inspection.mode_matches {
        if !ports.set_mode(mode)?.clean {
            return Err(PlaneError::Transport);
        }
        inspection = ports.inspect(mode)?;
        if !inspection.mode_matches {
            return Err(PlaneError::ConfigMismatch);
        }
    }
    state.applied_mode = Some(mode);
    state.verified_excluded = inspection.excluded_folders;
    // 6 先持久「已启动」再 RunOnce；回收后按退出码记退避（恢复逻辑只有 `recovery::mark_*` 一份），
    // 非 clean 本轮到此为止。
    state = recovery::mark_started(&state, ports.next_sequence()?);
    ports.save_state(state.clone())?;
    let run = ports.run_once()?;
    let reaped_at = ports.clock().unwrap_or(now_unix);
    state = recovery::mark_reaped(&state, run.clean, run.rc_one, reaped_at);
    ports.save_state(state.clone())?;
    if !run.clean {
        return Err(PlaneError::Transport);
    }
    // 强制恢复轮到此为止：mirror-remote 已把镜像拉齐，本轮不观察、不规划、不执行，
    // 尤其不能在审批为 bidirectional 时按「镜像为准」预解决冲突（评审 P2-1）。
    if decision.forced_recovery {
        return Ok(RoundOutcome::Recovered);
    }
    // 7 静止证据。
    ports.quiescent()?;
    // 10 上一批仍有未知终态：本轮不观察、不重放，等原键核对。
    if let Some(sequence) = ports.unresolved()? {
        return Ok(RoundOutcome::Unknown { sequence });
    }
    // 8＋9 两侧观察；MirrorRemote 下先试算，有冲突就按「镜像为准」预解决并重观察一次。
    let (mut input, _, _) = ports.observe(mode)?;
    if mode == SyncMode::MirrorRemote {
        let decision = plan(input.clone(), None).map_err(|_| PlaneError::Observation)?;
        if has_conflicts(&decision) && ports.preresolve(&decision)? {
            input = ports.observe(mode)?.0;
        }
    }
    // 10～12 预留、逐动作执行、闭合与保留期回收。
    ports.execute(input)
}

fn has_conflicts(decision: &Decision) -> bool {
    match decision {
        Decision::Ready(plan) => !plan.conflicts().is_empty(),
        Decision::FuseBlocked(blocked) => !blocked.conflicts.is_empty(),
    }
}

/// 经传输运行槽跑一次固定操作到闭合：prepare_once → Intent 审计 → start → wait 轮询
/// （超过 deadline 则 request_stop(DeadlineExceeded)）→ Outcome 审计。
///
/// # Errors
/// 传输槽拒绝返回 `Transport`；审计不可用返回 `Audit`。
pub fn run_headless(
    plane: &SyncPlane,
    operation: RunOperation,
    now_unix: i64,
) -> Result<HeadlessRun, PlaneError> {
    headless::run(plane, operation, now_unix)
}

/// 删除熔断的同一判据（>5 或 >原基线存活路径 10% 即阻断，等号可过）；
/// 引导期预解决删除也要过这一关，契约测试证明它与 `studiolo_sync_bridge::plan`
/// 对同一统计给出相同结论（sync-bridge 不在本轮写域，故这里不复用其私有函数）。
#[must_use]
pub fn deletion_fuse_blocked(deletions: u64, baseline_live_paths: u64) -> bool {
    let over_count = deletions > 5;
    let over_ratio = match (
        deletions.checked_mul(10000),
        baseline_live_paths.checked_mul(1000),
    ) {
        (Some(numerator), Some(denominator)) => numerator > denominator,
        _ => true,
    };
    over_count || over_ratio
}
