//! 一轮驱动的精确顺序（`docs/m2/sync-plane-design.md` 第 2 节）；每一步失败即停在原地，
//! 下一轮从持久状态续做，绝不补造成功、不重放 Issued。
//!
//! 本文件只编排；headless 运行经 `run_headless`，观察经 `witness`／`mirror`，执行经
//! `studiolo_sync_bridge::executor::execute`，全部端口由 `ports()` 装配。

use super::{PlaneError, SyncPlane};
use studiolo_sync_bridge::{Decision, Input, Inventory};
use studiolo_sync_transport::owned_run::{RunObservation, RunOperation, SyncMode};

/// 一轮的结论；每个变体都对应健康页第六节的一种状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundOutcome {
    /// 处于退避或恢复窗口，本轮什么都没跑。
    Skipped,
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
    /// 本次运行的最终观察。
    pub observation: RunObservation,
    /// 退出码是否为 0 且静止已证明。
    pub clean: bool,
    /// 退出码是否恰为 1（残留锁的典型值）。
    pub rc_one: bool,
}

/// 跑完整的一轮；`now_unix` 由调用方注入。
///
/// # Errors
/// 任一步失败返回对应固定分类；调用方只记健康，不重试。
pub fn run_round(plane: &SyncPlane, now_unix: i64) -> Result<RoundOutcome, PlaneError> {
    // B 轮 W3 实现，严格按下面的顺序，每步的停点见设计文档第 2 节表：
    // 1 recovery::load → effective_mode／退避判定（Backoff 即 Skipped）
    // 2 flush_transport_audit；活动槽存在则 recover(run_id)，未闭合即 Err(Transport)
    // 3 run_headless(InspectConfig) → sync_transport_config::verify 核模式／排除项／五类
    // 4 recovery::residue_guard（Blocked 即 Err(ResidueBlocked)）
    // 5 需要时 run_headless(SetMode(effective_mode)) 再回到 3
    // 6 recovery::mark_started → run_headless(RunOnce) → recovery::mark_reaped
    // 7 quiescence::OwnerQuiescence::take
    // 8 observe_both → 若 MirrorRemote 先 preresolve_conflicts 再重观察一次
    // 9 reserve_and_execute
    let _ = (plane, now_unix);
    Err(PlaneError::Unavailable)
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
    // B 轮 W3 实现。
    let _ = (plane, operation, now_unix);
    Err(PlaneError::Transport)
}

/// 两侧完整观察并经 `SyncScope::approve_union` 核准，组成冻结 `Input`。
///
/// # Errors
/// 任一侧不完整或范围未批准返回 `Observation`。
pub fn observe_both(
    plane: &SyncPlane,
    mode: SyncMode,
) -> Result<(Input, Inventory, Inventory), PlaneError> {
    // B 轮 W3 实现：canonical 经 witness::observe_canonical，镜像经 mirror::observe，
    // 序号取 BridgeState 游标 +1，Limits::conservative(max_file_bytes 取范围内最大 max_bytes)。
    let _ = (plane, mode);
    Err(PlaneError::Observation)
}

/// 引导期（MirrorRemote）冲突预解决（ADR-025 D6）：镜像为准，canonical 版本先保全到
/// `ops/bridge-conflicts/`；返回是否改动过 canonical（改过就必须重观察）。
///
/// # Errors
/// 保全或 CAS 失败返回 `Unavailable`；预解决删除越过熔断返回 `Fused`。
pub fn preresolve_conflicts(
    plane: &SyncPlane,
    decision: &Decision,
    now_unix: i64,
) -> Result<bool, PlaneError> {
    // B 轮 W3 实现（保全命名由 naming.rs 给，写入经 coordinator::sync_bridge）。
    let _ = (plane, decision, now_unix);
    Err(PlaneError::Unavailable)
}

/// `BridgeState::reserve` → `StateJournal` → `executor::execute` → 保留期回收。
///
/// # Errors
/// 熔断返回 `Fused`；有未知终态返回 `Unknown`；其余按端口失败分类。
pub fn reserve_and_execute(
    plane: &SyncPlane,
    input: Input,
    now_unix: i64,
) -> Result<RoundOutcome, PlaneError> {
    // B 轮 W3 实现。
    let _ = (plane, input, now_unix);
    Err(PlaneError::Unavailable)
}

/// 删除熔断的同一判据（>5 或 >原基线存活路径 10% 即阻断，等号可过）；
/// 引导期预解决删除也要过这一关，且 W3 须用契约测试证明它与 `studiolo_sync_bridge::plan`
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
