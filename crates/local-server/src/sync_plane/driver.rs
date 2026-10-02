//! 一轮驱动的精确顺序（`docs/m2/sync-plane-design.md` 第 2 节）；每一步失败即停在原地，
//! 下一轮从持久状态续做，绝不补造成功、不重放 Issued。
//!
//! 编排核心 `run_round_with` 只依赖端口 trait [`RoundPorts`]，便于用假端口做契约测试；
//! 真实端口在 `driver/real.rs`，headless 运行、观察、预解决、冲突保全、预留执行各在其子模块。
//! 关停只在安全停点生效（见 `halt` 的各调用点），执行器内部只在审计批次与完整动作边界停止。

pub mod content;
pub mod evidence;
pub mod execute;
pub mod headless;
pub mod observe;
pub mod preserve;
pub mod real;
pub mod reconcile;
pub mod resolve;

use super::recovery::{self, PlaneState};
use super::{PlaneError, SyncPlane};
use std::sync::atomic::AtomicBool;
use studiolo_host_config::sync_transport::TransportSyncMode;
use studiolo_sync_bridge::{Decision, Input, Inventory, plan};
use studiolo_sync_transport::owned_run::{RunId, RunObservation, SyncMode};

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

/// InspectConfig 复核的结论（核对顺序见 `headless::classify`）；四种组合都不一致时端口直接报
/// `ConfigMismatch`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
    /// headless 当前 `syncMode` 是否就是本轮应处的模式。
    pub mode_matches: bool,
    /// 本轮审批母本的排除文件夹集合（已排序）。
    pub excluded_folders: Vec<String>,
    /// headless 本次报的排除集合，即核对时命中的那份母本（已排序）。
    pub headless_excluded: Vec<String>,
    /// 审批已去掉、headless 仍排除、等清残留的文件夹（已排序）；非空＝排除项安全顺序进行中。
    pub pending_purge: Vec<String>,
}

/// 一轮驱动用到的全部外部能力；真实实现在 `real.rs`，契约测试用假端口记录调用序列。
pub trait RoundPorts {
    /// 读持久小状态（第 1 步）；撕裂即 `Storage`。
    fn state(&mut self) -> Result<PlaneState, PlaneError>;
    /// 补审计、回收旧槽并跑 InspectConfig 复核（第 2～3 步）。
    fn inspect(&mut self, expected: SyncMode) -> Result<Inspection, PlaneError>;
    /// 排除项守卫（第 4 步）：上次核验集合里有、当前没有的文件夹是否仍有镜像残留。
    /// 真实端口在 InspectConfig 刚闭合留下的静止票据下读视图，读完复验，作废即 `Quiescence`。
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
    /// 按规划结果回收冲突首次戳（第 9 步之前每轮都调）；假端口可不实现。
    fn retain_conflicts(&mut self, _decision: &Decision) -> Result<(), PlaneError> {
        Ok(())
    }
    /// 引导期冲突预解决（第 9 步）；返回是否改动过 canonical。
    fn preresolve(&mut self, decision: &Decision) -> Result<bool, PlaneError>;
    /// 双向冲突保全（第 9 步）：双方现存版本各保全一份到 `ops/bridge-conflicts/`，同一冲突跨轮只保全一次；
    /// 返回本轮新保全的冲突数。
    fn preserve_conflicts(&mut self, decision: &Decision) -> Result<u64, PlaneError>;
    /// 预留、逐动作执行、闭合与保留期回收（第 10～12 步）。
    fn execute(&mut self, input: Input) -> Result<RoundOutcome, PlaneError>;
    /// 关停是否已请求；真即本轮在最近的安全停点以 `Stopping` 结束。端口不提供即从不关停。
    fn stopping(&mut self) -> bool {
        false
    }
}

/// 跑完整的一轮；`now_unix` 由调用方注入，`stopping` 是 `SyncPlane::drive_until` 的关停标志。
///
/// # Errors
/// 任一步失败返回对应固定分类；调用方只记健康，不重试。
pub fn run_round(
    plane: &SyncPlane,
    now_unix: i64,
    stopping: &AtomicBool,
) -> Result<RoundOutcome, PlaneError> {
    let approved = approved_mode(plane.approval().headless_mode());
    let mut ports = real::RealPorts::open(plane, now_unix, stopping)?;
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
/// 任一步失败返回对应固定分类；关停已请求时在最近的安全停点返回 `Stopping`。
pub fn run_round_with(
    ports: &mut dyn RoundPorts,
    approved: SyncMode,
    now_unix: i64,
) -> Result<RoundOutcome, PlaneError> {
    // 1 持久小状态与恢复判定：退避窗口内什么都不跑。
    let mut state = ports.state()?;
    if state.pending_purge.is_some() {
        return Err(PlaneError::Audit);
    }
    let decision = recovery::decide(&state, approved, now_unix);
    if decision.backoff {
        return Ok(RoundOutcome::Skipped);
    }
    let mode = decision.effective_mode;
    // 2～4 补审计、回收旧槽、InspectConfig 复核、记下 headless 现状、排除项守卫。
    let mut inspection = inspect_guarded(ports, &mut state, mode)?;
    // 5 模式不符才 SetMode，然后按同样逻辑回到 3 复核；复核通过后才记 applied_mode。
    if !inspection.mode_matches {
        halt(ports)?;
        let run = ports.set_mode(mode)?;
        if !run.clean {
            return Err(run.failure());
        }
        inspection = inspect_guarded(ports, &mut state, mode)?;
        if !inspection.mode_matches {
            return Err(PlaneError::ConfigMismatch);
        }
    }
    state.applied_mode = Some(mode);
    state.verified_excluded = inspection.excluded_folders;
    // 6 先持久「已启动」再 RunOnce；回收后按退出码记退避（恢复逻辑只有 `recovery::mark_*` 一份），
    // 干净与否都先落盘；关停已请求（运行已被 `request_stop` 停下）即到此为止，非 clean 同样到此为止。
    halt(ports)?;
    state = recovery::mark_started(&state, ports.next_sequence()?);
    ports.save_state(state.clone())?;
    let run = ports.run_once()?;
    let reaped_at = ports.clock().unwrap_or(now_unix);
    state = recovery::mark_reaped(&state, run.clean, run.rc_one, reaped_at);
    ports.save_state(state.clone())?;
    halt(ports)?;
    if !run.clean {
        return Err(run.failure());
    }
    // 强制恢复轮到此为止：mirror-remote 已把镜像拉齐，本轮不观察、不规划、不执行，
    // 尤其不能在审批为 bidirectional 时按「镜像为准」预解决冲突（评审 P2-1）。
    if decision.forced_recovery {
        return Ok(RoundOutcome::Recovered);
    }
    // 7 静止证据。
    halt(ports)?;
    ports.quiescent()?;
    // 10 上一批仍有未知终态：本轮不观察、不重放，等原键核对。
    if let Some(sequence) = ports.unresolved()? {
        return Ok(RoundOutcome::Unknown { sequence });
    }
    // 8＋9 两侧观察后一律试算。MirrorRemote 有冲突：按「镜像为准」预解决并重观察一次；
    // Bidirectional 有冲突：双方版本先保全，再照常执行（执行器对冲突路径不动、闭合时不推进基线）；
    // 熔断冻结的批次带冲突同样先保全，再交执行段报 `Fused`。
    halt(ports)?;
    let (mut input, _, _) = ports.observe(mode)?;
    let decision = plan(input.clone(), None).map_err(|_| PlaneError::Observation)?;
    ports.retain_conflicts(&decision)?;
    if has_conflicts(&decision) {
        match mode {
            SyncMode::MirrorRemote => {
                if ports.preresolve(&decision)? {
                    halt(ports)?;
                    input = ports.observe(mode)?.0;
                }
            }
            SyncMode::Bidirectional => {
                ports.preserve_conflicts(&decision)?;
            }
        }
    }
    // 10～12 预留、逐动作执行、闭合与保留期回收。
    halt(ports)?;
    ports.execute(input)
}

/// 第 2～4 步：InspectConfig 复核 → headless 报的排除集合有变就落盘（守卫拦住时也要留下 headless
/// 现状，供 purge 校验）→ 排除项守卫 → 残留已清但 headless 仍排除审批已去掉的文件夹即
/// `ConfigMismatch`（该由管理员在 headless 侧清掉排除项了）。
fn inspect_guarded(
    ports: &mut dyn RoundPorts,
    state: &mut PlaneState,
    mode: SyncMode,
) -> Result<Inspection, PlaneError> {
    halt(ports)?;
    let inspection = ports.inspect(mode)?;
    if state.headless_excluded != inspection.headless_excluded {
        state.headless_excluded = inspection.headless_excluded.clone();
        ports.save_state(state.clone())?;
    }
    if ports.residue_blocked(&state.verified_excluded, &inspection.excluded_folders)? {
        return Err(PlaneError::ResidueBlocked);
    }
    if !inspection.pending_purge.is_empty() {
        return Err(PlaneError::ConfigMismatch);
    }
    Ok(inspection)
}

/// 安全停点：关停已请求就以 `Stopping` 结束本轮，不再发起新的运行、观察或执行。
fn halt(ports: &mut dyn RoundPorts) -> Result<(), PlaneError> {
    if ports.stopping() {
        return Err(PlaneError::Stopping);
    }
    Ok(())
}

fn has_conflicts(decision: &Decision) -> bool {
    match decision {
        Decision::Ready(plan) => !plan.conflicts().is_empty(),
        Decision::FuseBlocked(blocked) => !blocked.conflicts.is_empty(),
    }
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
