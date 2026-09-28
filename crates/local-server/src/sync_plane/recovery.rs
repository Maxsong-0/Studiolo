//! 非正常退出恢复协议与退避（ADR-025 D9）、排除项守卫（D10）的持久小状态
//! `<plane_root>/plane-state.json`：固定名、原子替换、失败关闭（编解码在 `state.rs`）；
//! `plane.lock` 是平面排他租约。
//!
//! 规则：上次未干净回收（含断电）→ 先 `SetMode(MirrorRemote)` 加一次 `RunOnce`，
//! 成功后才允许回到审批模式；任何非 `Code(0)`＋静止确认的运行之后下一次 `RunOnce`
//! 至少等 5 s；rc=1 连续出现按 5/10/20/40/80/160/300 s 退避（只看退出码，不解析 stderr）。
//!
//! 本文件的真实 IO 只有两处固定形态：逐段 NOFOLLOW 打开一个目录路径（平面根、视图根）
//! 与 `flock` 租约；建文件、整读、原子替换、节点分类与目录枚举都借 `mirror::stage`
//! 里只认已有描述符的助手。
//!
//! 版本 1 之后追加的字段（`headless_excluded`、`conflict_stamps`）缺省为空、为空时不写出，
//! 只在非默认时进校验和：旧文件照常解码、校验和不变，不需要重建（兼容规则见 `state.rs`）。

mod residue;
mod state;

pub use residue::residue_guard;

use super::PlaneError;
use super::mirror::stage;
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{CWD, FlockOperation, Mode, OFlags, Stat, flock, fstat, openat};
use rustix::io::Errno;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Component, Path};
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
/// 租约文件字节上限（设计文档第 5 节）。
const LOCK_MAX_BYTES: usize = 4 * 1024;
/// `conflict_stamps` 的条目上限，也是一轮最多保全的冲突数（`driver::preserve`）。
pub const MAX_CONFLICT_STAMPS: usize = 512;

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
    /// headless 最近一次核对时报的排除集合（已排序）；守卫拦住的轮次也记。
    /// `sync-purge-residue` 只清其中的文件夹：headless 已不再排除的残留会被当成新内容上传。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headless_excluded: Vec<String>,
    /// 双向冲突保全戳：冲突身份摘要（64 位小写十六进制）→ 首次保全的 UTC 分钟戳（12 位数字）；
    /// 只留当前冲突，至多 [`MAX_CONFLICT_STAMPS`] 条。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub conflict_stamps: BTreeMap<String, String>,
    /// 域分隔 sha256（小写十六进制）。
    pub checksum: String,
}

impl PlaneState {
    /// 首次运行的状态：视为上次已干净回收、无退避、无已核验排除项、没有记下的 headless 现状与冲突戳。
    #[must_use]
    pub fn first_run() -> Self {
        state::seal(Self {
            version: state::VERSION,
            last_run_clean: true,
            started_sequence: None,
            last_exit_unix: None,
            consecutive_rc1: 0,
            not_before_unix: None,
            verified_excluded: Vec::new(),
            applied_mode: None,
            headless_excluded: Vec::new(),
            conflict_stamps: BTreeMap::new(),
            checksum: String::new(),
        })
    }
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

/// 记录「已启动」：`sequence` 是本次 RunOnce 的运行序号；断电后重开见到它即视为未干净回收。
#[must_use]
pub fn mark_started(state: &PlaneState, sequence: u64) -> PlaneState {
    state::seal(PlaneState {
        started_sequence: Some(sequence),
        ..state.clone()
    })
}

/// 记录回收结果：干净 → 清零退避；rc=1 → 连续计数加一并按阶梯退避；
/// 其它非干净退出 → 连续计数归零、至少等 `MIN_GAP_SECONDS`。
#[must_use]
pub fn mark_reaped(state: &PlaneState, clean: bool, rc_one: bool, now_unix: i64) -> PlaneState {
    let mut next = state.clone();
    next.started_sequence = None;
    next.last_run_clean = clean;
    next.last_exit_unix = Some(now_unix);
    let gap = if clean {
        next.consecutive_rc1 = 0;
        None
    } else if rc_one {
        next.consecutive_rc1 = next.consecutive_rc1.saturating_add(1);
        Some(backoff_seconds(next.consecutive_rc1))
    } else {
        next.consecutive_rc1 = 0;
        Some(MIN_GAP_SECONDS)
    };
    next.not_before_unix =
        gap.map(|seconds| now_unix.saturating_add(i64::try_from(seconds).unwrap_or(i64::MAX)));
    state::seal(next)
}

/// 已打开的持久状态存储；持有平面根 FD 与排他租约，实例活着就一直占着 `plane.lock`，
/// 析构时由 [`Lease`] 显式解锁。
pub struct PlaneStateStore {
    root: OwnedFd,
    /// 只为占着租约而持有：活着即持锁，析构即显式解锁。
    _lease: Lease,
    state: PlaneState,
}

impl PlaneStateStore {
    /// 打开 `<plane_root>/plane-state.json`；文件不存在视为首次运行（`last_run_clean=true`，
    /// `verified_excluded` 空）；撕裂／未知版本／校验和不符一律失败关闭。
    ///
    /// # Errors
    /// 目录权限、租约被占或文件损坏返回 `Storage`。
    pub fn open(plane_root: &Path) -> Result<Self, PlaneError> {
        let euid = stage::caller()?;
        let (root, stat) = directory_path(plane_root).map_err(|_| PlaneError::Storage)?;
        if !stage::private_directory(&stat, euid) {
            return Err(PlaneError::Storage);
        }
        let (lease, lease_stat) =
            stage::create_at(root.as_fd(), PLANE_LOCK, false).map_err(|_| PlaneError::Storage)?;
        if stage::private_regular(&lease_stat, euid, LOCK_MAX_BYTES).is_none() {
            return Err(PlaneError::Storage);
        }
        flock(&lease, FlockOperation::NonBlockingLockExclusive).map_err(|_| PlaneError::Storage)?;
        // 一拿到锁就交给守卫：后面任何一步失败提前返回，也在析构时显式解锁（评审 P3-4）。
        let lease = Lease(lease);
        let existing =
            stage::open_regular_at(root.as_fd(), PLANE_STATE_FILE, euid, PLANE_STATE_MAX_BYTES)
                .map_err(|_| PlaneError::Storage)?;
        let state = match existing {
            Some((file, length)) => state::decode(
                &stage::read_fully(file.as_fd(), length).map_err(|_| PlaneError::Storage)?,
            )?,
            None => PlaneState::first_run(),
        };
        Ok(Self {
            root,
            _lease: lease,
            state,
        })
    }

    /// 当前状态。
    #[must_use]
    pub fn state(&self) -> &PlaneState {
        &self.state
    }

    /// 完整替换持久状态（版本与校验和由这里补齐，两个排除集合排序去重）；
    /// 写前按解码同一套规则自检，畸形状态不落盘，免得下次打开才失败关闭。
    ///
    /// # Errors
    /// 冲突戳畸形、超过预算或 IO 失败返回 `Storage`。
    pub fn save(&mut self, next: PlaneState) -> Result<(), PlaneError> {
        let next = state::seal(next);
        state::validate(&next)?;
        let bytes = serde_json::to_vec(&next).map_err(|_| PlaneError::Storage)?;
        if bytes.len() > PLANE_STATE_MAX_BYTES {
            return Err(PlaneError::Storage);
        }
        stage::replace_file(
            self.root.as_fd(),
            PLANE_STATE_STAGE_FILE,
            PLANE_STATE_FILE,
            &bytes,
        )
        .map_err(|_| PlaneError::Storage)?;
        self.state = next;
        Ok(())
    }
}

/// `plane.lock` 的排他租约。flock 按打开文件描述计：同进程里并发的 fork 可能在 exec 之前一直持有
/// 这个描述符，只等本 FD 关闭，锁会多活到子进程 exec；析构时显式解锁立即结束租约。Drop 不能报告错误。
struct Lease(OwnedFd);

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = flock(&self.0, FlockOperation::Unlock);
    }
}

/// 逐段 NOFOLLOW 打开一个绝对目录路径（平面根或镜像视图根）；任一段是链接或不是目录即失败。
fn directory_path(path: &Path) -> rustix::io::Result<(OwnedFd, Stat)> {
    if !path.is_absolute() {
        return Err(Errno::INVAL);
    }
    let flags =
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let mut handle: Option<OwnedFd> = None;
    for component in path.components() {
        let name = match component {
            Component::RootDir => OsStr::new("/"),
            Component::Normal(name) => name,
            _ => return Err(Errno::INVAL),
        };
        let dir = handle.as_ref().map_or(CWD, AsFd::as_fd);
        handle = Some(openat(dir, name, flags, Mode::empty())?);
    }
    let handle = handle.ok_or(Errno::INVAL)?;
    let stat = fstat(&handle)?;
    Ok((handle, stat))
}

#[cfg(test)]
#[path = "../../tests/sync_plane_private/lease.rs"]
mod lease_release;
