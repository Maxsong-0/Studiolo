//! 镜像视图（idmapped bind，ADR-025 D7／D8）的观察、正文读出与安装。
//!
//! 只在静止窗口内读写镜像；下载是原地写入，运行期间最终路径可见半截内容（P2 结论 2）。
//! 所有真实 IO 只落在 `stage.rs`（外 stage）与本文件标记的两处固定形态：
//! 观察走 NOFOLLOW 目录遍历＋流式 sha256；安装走 stage 对象 → `renameat` 进视图。

pub mod stage;

use super::PlaneError;
use std::path::{Path, PathBuf};
use studiolo_core::VaultPath;
use studiolo_host_config::SyncObservationSelection;
use studiolo_observability::quota::SyncUsage;
use studiolo_sync_bridge::executor::{MirrorContent, MirrorObjectSink, ObjectId, TransferFault};
use studiolo_sync_bridge::{Binding, FileVersion, Inventory, Side};

/// 已核验的镜像视图根：路径加取证时刻的 `(dev, ino)`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MirrorView {
    root: PathBuf,
    identity: (u64, u64),
}

impl MirrorView {
    /// 由静止票据给出的视图根与身份构造；每次使用前复验身份未变。
    #[must_use]
    pub fn new(root: PathBuf, identity: (u64, u64)) -> Self {
        Self { root, identity }
    }

    /// 视图根。
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 取证时刻的 `(dev, ino)`。
    #[must_use]
    pub fn identity(&self) -> (u64, u64) {
        self.identity
    }
}

/// 一次镜像完整观察的结果：清单加字节合计（健康页 `sync-quota` 用）。
pub struct MirrorObservation {
    /// 完整清单；缺失显式 `Missing`。
    pub inventory: Inventory,
    /// 范围内对象数与字节数。
    pub usage: SyncUsage,
}

/// 在静止窗口内完整观察镜像视图：按 `selection` 遍历，逐文件流式 sha256，
/// 链接／特殊文件／越权名字任一出现即 `complete=false`。
///
/// # Errors
/// 视图身份漂移或遍历失败返回 `Observation`。
pub fn observe(
    view: &MirrorView,
    selection: &SyncObservationSelection,
    binding: &Binding,
    sequence: u64,
    required: &[VaultPath],
) -> Result<MirrorObservation, PlaneError> {
    // B 轮 W4 实现。revision 取 sha256 本身（镜像没有 Broker revision）。
    let _ = (view, selection, binding, sequence, required, Side::Mirror);
    Err(PlaneError::Observation)
}

/// 冻结版本的正文来源：canonical 侧经 `coordinator::sync_bridge` 读，镜像侧经视图读；
/// 打开时核对 sha256／bytes 与冻结版本一致，不一致即 `ContentMismatch`。
pub struct PlaneContent<'a> {
    view: &'a MirrorView,
    plane: &'a super::SyncPlane,
    open: Option<OpenSource>,
}

struct OpenSource {
    side: Side,
    path: VaultPath,
    version: FileVersion,
    offset: u64,
}

impl<'a> PlaneContent<'a> {
    /// 组装正文来源。
    #[must_use]
    pub fn new(plane: &'a super::SyncPlane, view: &'a MirrorView) -> Self {
        Self {
            view,
            plane,
            open: None,
        }
    }
}

impl MirrorContent for PlaneContent<'_> {
    fn open(
        &mut self,
        side: Side,
        path: &VaultPath,
        version: &FileVersion,
    ) -> Result<(), TransferFault> {
        // B 轮 W4 实现（canonical 侧读经 plane.workbench 的接缝，正文 ≤ 256 MiB 分帧）。
        let _ = (self.view, self.plane);
        self.open = Some(OpenSource {
            side,
            path: path.clone(),
            version: version.clone(),
            offset: 0,
        });
        Err(TransferFault::Unavailable)
    }

    fn next_frame(&mut self, buffer: &mut [u8]) -> Result<usize, TransferFault> {
        // B 轮 W4 实现。
        let _ = (
            buffer,
            self.open
                .as_ref()
                .map(|open| (open.side, &open.path, &open.version, open.offset)),
        );
        Err(TransferFault::Unavailable)
    }
}

/// 外 stage 的接收出口：`begin` 建临时件，`accept` 追加，`finish` 按地址 renameat 封口。
pub struct StageSink<'a> {
    stage: &'a mut stage::StageStore,
}

impl<'a> StageSink<'a> {
    /// 绑定到已打开的外 stage。
    #[must_use]
    pub fn new(stage: &'a mut stage::StageStore) -> Self {
        Self { stage }
    }
}

impl MirrorObjectSink for StageSink<'_> {
    fn begin(&mut self) -> Result<(), TransferFault> {
        self.stage.begin().map_err(|_| TransferFault::Unavailable)
    }

    fn accept(&mut self, frame: &[u8]) -> Result<(), TransferFault> {
        self.stage
            .accept(frame)
            .map_err(|_| TransferFault::Unavailable)
    }

    fn finish(&mut self, id: ObjectId) -> Result<(), TransferFault> {
        self.stage
            .finish(id)
            .map_err(|_| TransferFault::Unavailable)
    }

    fn discard(&mut self) -> Result<(), TransferFault> {
        self.stage.discard().map_err(|_| TransferFault::Unavailable)
    }
}

/// 把已封口的 stage 对象原子安装进视图：先复验视图身份与目标当前版本
/// （`target_expected`，缺失即须不存在），再 `renameat` 进目标路径并复验。
///
/// # Errors
/// 目标版本不符返回 `Observation`（执行器记 NoEffect）；安装后复验失败返回 `Unknown`。
pub fn install(
    view: &MirrorView,
    stage: &mut stage::StageStore,
    object: ObjectId,
    target: &VaultPath,
    target_expected: Option<&FileVersion>,
) -> Result<FileVersion, PlaneError> {
    // B 轮 W4 实现。
    let _ = (view, stage, object, target, target_expected);
    Err(PlaneError::Unavailable)
}

/// 删除视图内一个路径：先把当前字节按地址保全进 stage（删除前保全，7 天保留），再 unlink。
///
/// # Errors
/// 目标版本不符返回 `Observation`；保全失败返回 `Storage`。
pub fn remove(
    view: &MirrorView,
    stage: &mut stage::StageStore,
    target: &VaultPath,
    target_expected: &FileVersion,
    plan_sequence: u64,
    now_unix: i64,
) -> Result<ObjectId, PlaneError> {
    // B 轮 W4 实现；`plan_sequence`／`now_unix` 只用于 `StageStore::preserve` 的保全登记。
    let _ = (
        view,
        stage,
        target,
        target_expected,
        plan_sequence,
        now_unix,
    );
    Err(PlaneError::Unavailable)
}
