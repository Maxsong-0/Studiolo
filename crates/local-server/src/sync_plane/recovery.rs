//! 非正常退出恢复协议与退避（ADR-025 D9）、排除项守卫（D10）的持久小状态
//! `<plane_root>/plane-state.json`：固定名、原子替换、失败关闭。
//!
//! 规则：上次未干净回收（含断电）→ 先 `SetMode(MirrorRemote)` 加一次 `RunOnce`，
//! 成功后才允许回到审批模式；任何非 `Code(0)`＋静止确认的运行之后下一次 `RunOnce`
//! 至少等 5 s；rc=1 连续出现按 5/10/20/40/80/160/300 s 退避（只看退出码，不解析 stderr）。

use super::PlaneError;
use std::path::Path;
use studiolo_sync_transport::owned_run::SyncMode;

/// 持久状态文件名。
pub const PLANE_STATE_FILE: &str = "plane-state.json";
/// 唯一暂存件名。
pub const PLANE_STATE_STAGE_FILE: &str = "plane-state.json.new";
/// 平面排他租约文件名。
pub const PLANE_LOCK: &str = "plane.lock";
/// 状态文件字节上限。
pub const PLANE_STATE_MAX_BYTES: usize = 64 * 1024;
/// 非干净退出后的最短间隔。
pub const MIN_GAP_SECONDS: u64 = 5;
/// rc=1 连续出现时的退避阶梯（秒）；超过最后一档保持 300。
pub const BACKOFF_SECONDS: [u64; 7] = [5, 10, 20, 40, 80, 160, 300];

/// 持久小状态；`serde(deny_unknown_fields)`，版本固定 1。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaneState {
    /// 固定为 1。
    pub version: u8,
    /// 上次 RunOnce 是否已干净回收（`Code(0)` 且静止确认）；`false` 或「已启动未回收」都算未干净。
    pub last_run_clean: bool,
    /// 已启动但尚未记录回收的运行序号；断电后重开时 `Some` 即视为未干净回收。
    pub started_sequence: Option<u64>,
    /// 上次退出时刻（Unix 秒）。
    pub last_exit_unix: Option<i64>,
    /// 连续 rc=1 次数。
    pub consecutive_rc1: u32,
    /// 下一次 RunOnce 不早于此刻（Unix 秒）。
    pub not_before_unix: Option<i64>,
    /// 上次核验通过的排除文件夹集合（已排序）。
    pub verified_excluded: Vec<String>,
    /// 上次成功应用的镜像模式。
    pub applied_mode: Option<SyncMode>,
    /// 域分隔 sha256（小写十六进制）。
    pub checksum: String,
}

/// 本轮应采用的模式与是否处于退避。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryDecision {
    /// 本轮 headless 应处的模式。
    pub effective_mode: SyncMode,
    /// 是否为强制恢复轮（上次未干净回收）。
    pub forced_recovery: bool,
    /// 本轮是否因退避而跳过 RunOnce。
    pub backoff: bool,
}

/// 纯判定：由持久状态与审批模式给出本轮决定。
#[must_use]
pub fn decide(state: &PlaneState, approved: SyncMode, now_unix: i64) -> RecoveryDecision {
    let unclean = !state.last_run_clean || state.started_sequence.is_some();
    RecoveryDecision {
        effective_mode: if unclean {
            SyncMode::MirrorRemote
        } else {
            approved
        },
        forced_recovery: unclean,
        backoff: state.not_before_unix.is_some_and(|until| now_unix < until),
    }
}

/// rc=1 连续 `count` 次后的退避秒数；`count` 从 1 起，超过阶梯长度取最后一档。
#[must_use]
pub fn backoff_seconds(count: u32) -> u64 {
    let index = usize::try_from(count.saturating_sub(1)).unwrap_or(usize::MAX);
    BACKOFF_SECONDS
        .get(index)
        .copied()
        .unwrap_or(BACKOFF_SECONDS[BACKOFF_SECONDS.len() - 1])
}

/// 排除项守卫（D10）：`previous` 里有、`current` 里没有的文件夹，若镜像视图里仍有残留，
/// 返回第一个残留文件夹名；调用方据此拒绝 RunOnce。
///
/// # Errors
/// 视图不可读返回 `Unavailable`。
pub fn residue_guard(
    previous: &[String],
    current: &[String],
    view_root: &Path,
) -> Result<Option<String>, PlaneError> {
    // B 轮 W4 实现（只 stat 目录是否存在且非空，不递归读正文）。
    let _ = (previous, current, view_root);
    Err(PlaneError::Unavailable)
}

/// 已打开的持久状态存储。
pub struct PlaneStateStore {
    state: PlaneState,
}

impl PlaneStateStore {
    /// 打开 `<plane_root>/plane-state.json`；文件不存在视为首次运行（`last_run_clean=true`，
    /// `verified_excluded` 空）；撕裂／未知版本／校验和不符一律失败关闭。
    ///
    /// # Errors
    /// 目录权限或文件损坏返回 `Storage`。
    pub fn open(plane_root: &Path) -> Result<Self, PlaneError> {
        // B 轮 W4 实现（真实 IO 只许在本文件的 open／save 两处，形态照 deadman-state.json）。
        let _ = (
            plane_root,
            PLANE_STATE_FILE,
            PLANE_STATE_STAGE_FILE,
            PLANE_LOCK,
        );
        Err(PlaneError::Storage)
    }

    /// 当前状态。
    #[must_use]
    pub fn state(&self) -> &PlaneState {
        &self.state
    }

    /// 完整替换持久状态。
    ///
    /// # Errors
    /// IO 失败返回 `Storage`。
    pub fn save(&mut self, next: PlaneState) -> Result<(), PlaneError> {
        // B 轮 W4 实现。
        let _ = (next, PLANE_STATE_MAX_BYTES, MIN_GAP_SECONDS);
        Err(PlaneError::Storage)
    }
}
