//! canonical 侧正文搬运的共用形态：≤ 4 MiB 整篇条件写，更大走只读来源流式写；
//! D5／D6 的 canonical 版本保全也从这里走。本文件不持有 Broker 类型，只经接缝。

use super::super::mirror::stage::{OBJECTS_DIR, STAGE_DIR, StageStore};
use super::super::mirror::{MirrorView, PlaneContent};
use super::super::witness::seam_error;
use super::super::{PlaneError, SyncPlane, naming, quiescence};
use super::evidence::{hex, revision_digest, sha256};
use crate::coordinator::sync_bridge::{BridgePrecondition, BridgeSeamError};
use std::cell::RefCell;
use studiolo_core::VaultPath;
use studiolo_sync_bridge::executor::{MirrorContent, ObjectId};
use studiolo_sync_bridge::{Binding, FileVersion, Side};
use studiolo_sync_transport::owned_run::{QuiescentMirror, SyncMode};
use studiolo_vault_broker::{BlobIdentity, BlobSource, MutationReceipt};

/// 整篇条件写的正文上限；更大的对象走流式写。
pub const INLINE_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// 逐帧读正文的固定帧长（与 MirrorCAS 相同）。
pub const FRAME_BYTES: usize = 64 * 1024;

/// 一轮里各步共享的上下文：平面、静止票据与它证明的视图、外 stage、绑定与模式。
pub struct RoundContext<'a> {
    /// 所属平面。
    pub plane: &'a SyncPlane,
    /// 第 7 步取得的静止票据；每次读写镜像前都先经传输槽 `verify_quiescent` 复验它。
    pub ticket: &'a QuiescentMirror,
    /// 静止票据证明的视图根。
    pub view: MirrorView,
    /// 本轮打开的外 stage。
    pub stage: &'a RefCell<StageStore>,
    /// Bridge 持久状态的绑定。
    pub binding: Binding,
    /// 本轮 headless 所处模式。
    pub mode: SyncMode,
    /// 本轮时刻（Unix 秒）。
    pub now_unix: i64,
}

/// 从镜像条件写 canonical 的失败。`Source`：镜像上的冻结版本读不出或已不是它（票据作废、文件变了或没了），
/// 本次调用还没碰接缝——首次执行据此记「确定未安装」，重开核对却据此什么也证明不了（原请求重建不出，
/// 原键下可能早已安装），只能保持未知（评审 2026-09-28 P1-1）。`Seam`：接缝本身的分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorWriteError {
    /// 镜像源读不出或已变。
    Source,
    /// 接缝返回的分类。
    Seam(BridgeSeamError),
}

impl MirrorWriteError {
    /// 在不区分两者的语境（预解决）里当接缝分类用：镜像源读不出即还没碰 Broker。
    #[must_use]
    pub fn seam(self) -> BridgeSeamError {
        match self {
            Self::Source => BridgeSeamError::NotAttempted,
            Self::Seam(error) => error,
        }
    }
}

/// 经视图逐帧读出镜像上冻结版本的正文并核对摘要与字节数；读之前先复验静止票据。
///
/// # Errors
/// 静止票据已作废返回 `Quiescence`；视图不可用返回 `Unavailable`；正文与冻结版本不符返回 `Observation`。
pub fn read_mirror_bytes(
    context: &RoundContext<'_>,
    path: &VaultPath,
    version: &FileVersion,
) -> Result<Vec<u8>, PlaneError> {
    quiescence::verify(context.plane, context.ticket)?;
    let mut content = PlaneContent::new(context.plane, &context.view);
    content
        .open(Side::Mirror, path, version)
        .map_err(|_| PlaneError::Unavailable)?;
    let mut bytes = Vec::new();
    let mut buffer = vec![0u8; FRAME_BYTES];
    loop {
        let read = content
            .next_frame(&mut buffer)
            .map_err(|_| PlaneError::Unavailable)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(buffer.get(..read).ok_or(PlaneError::Unavailable)?);
        if bytes.len() as u64 > version.bytes {
            return Err(PlaneError::Observation);
        }
    }
    if bytes.len() as u64 != version.bytes || sha256(&bytes) != version.sha256 {
        return Err(PlaneError::Observation);
    }
    Ok(bytes)
}

/// 经接缝读出 canonical 上冻结版本的正文；revision、摘要与字节数任一不符即视为已变。
///
/// # Errors
/// 写者不可用返回 `Unavailable`；文件已不是冻结版本返回 `Observation`。
pub fn read_canonical_bytes(
    plane: &SyncPlane,
    path: &VaultPath,
    version: &FileVersion,
) -> Result<Vec<u8>, PlaneError> {
    let read = plane
        .workbench()
        .bridge_read(path, version.bytes)
        .map_err(|error| match error {
            BridgeSeamError::Rejected => PlaneError::Observation,
            other => seam_error(other),
        })?;
    if read.content.len() as u64 != version.bytes
        || sha256(&read.content) != version.sha256
        || revision_digest(read.revision) != version.revision
    {
        return Err(PlaneError::Observation);
    }
    Ok(read.content)
}

/// 把已在内存里的正文条件写进 canonical：小对象整篇写，大对象先落进外 stage 再流式写。
///
/// # Errors
/// 分类同接缝：`Rejected`／`Unknown`／`Audit`／`Unavailable`。
pub fn write_bytes(
    plane: &SyncPlane,
    stage: &RefCell<StageStore>,
    path: &VaultPath,
    bytes: Vec<u8>,
    precondition: BridgePrecondition,
    key: &str,
) -> Result<MutationReceipt, BridgeSeamError> {
    if bytes.len() as u64 <= INLINE_MAX_BYTES {
        return plane
            .workbench()
            .bridge_write(path, bytes, precondition, key);
    }
    let (identity, source) = stage_source(plane, stage, &bytes)?;
    plane
        .workbench()
        .bridge_write_stream(path, source, identity, precondition, key)
}

/// 把镜像上的冻结版本条件写进 canonical：小对象经视图逐帧读后整篇写，大对象以视图为只读来源流式写。
///
/// # Errors
/// 镜像源读不出或已不是冻结版本（含静止票据作废）返回 `Source`；其余是接缝分类。
pub fn write_from_mirror(
    context: &RoundContext<'_>,
    path: &VaultPath,
    version: &FileVersion,
    precondition: BridgePrecondition,
    key: &str,
) -> Result<MutationReceipt, MirrorWriteError> {
    let workbench = context.plane.workbench();
    if version.bytes <= INLINE_MAX_BYTES {
        let bytes =
            read_mirror_bytes(context, path, version).map_err(|_| MirrorWriteError::Source)?;
        return workbench
            .bridge_write(path, bytes, precondition, key)
            .map_err(MirrorWriteError::Seam);
    }
    // 流式写由写者按需从视图读：交出来源前复验静止；读取本身发生在接缝之内，失败归接缝分类。
    quiescence::verify(context.plane, context.ticket).map_err(|_| MirrorWriteError::Source)?;
    let source = BlobSource::open(context.view.root(), path.clone())
        .map_err(|_| MirrorWriteError::Source)?;
    let identity = BlobIdentity {
        bytes: version.bytes,
        sha256: hex(&version.sha256),
    };
    workbench
        .bridge_write_stream(path, source, identity, precondition, key)
        .map_err(MirrorWriteError::Seam)
}

/// 把 canonical 上的冻结版本保全成范围外副本（D5 `ops/bridge-rollback/`、D6 `ops/bridge-conflicts/`）。
/// 副本名里的分钟戳由调用方给：同一幂等键必须永远对应同一副本路径，否则重试会以新路径撞旧键。
///
/// # Errors
/// 副本命名失败、原文件已变或写者不可用返回对应分类；同键重试直接命中原回执。
pub fn preserve_canonical(
    context: &RoundContext<'_>,
    directory: &str,
    path: &VaultPath,
    version: &FileVersion,
    stamp: &str,
    key: &str,
) -> Result<(), PlaneError> {
    let copy = naming::copy_path(directory, path, naming::CANONICAL_DEVICE, stamp)
        .ok_or(PlaneError::Unavailable)?;
    let bytes = read_canonical_bytes(context.plane, path, version)?;
    write_bytes(
        context.plane,
        context.stage,
        &copy,
        bytes,
        BridgePrecondition::MustNotExist,
        key,
    )
    .map(|_| ())
    .map_err(seam_error)
}

/// 把正文按地址落进外 stage，并以 stage 目录为根建只读来源；对象留作孤儿等保留期回收。
fn stage_source(
    plane: &SyncPlane,
    stage: &RefCell<StageStore>,
    bytes: &[u8],
) -> Result<(BlobIdentity, BlobSource), BridgeSeamError> {
    let digest = sha256(bytes);
    {
        let mut store = stage.borrow_mut();
        let staged = store.begin().and_then(|()| {
            for chunk in bytes.chunks(FRAME_BYTES) {
                store.accept(chunk)?;
            }
            store.finish(ObjectId::from_digest(digest))
        });
        if staged.is_err() {
            let _ = store.discard();
            return Err(BridgeSeamError::Unavailable);
        }
    }
    let root = plane.plane_root().join(STAGE_DIR);
    let logical = VaultPath::parse(format!("{OBJECTS_DIR}/{}", hex(&digest)))
        .map_err(|_| BridgeSeamError::Unavailable)?;
    let source = BlobSource::open(&root, logical).map_err(|_| BridgeSeamError::Unavailable)?;
    let identity = BlobIdentity {
        bytes: bytes.len() as u64,
        sha256: hex(&digest),
    };
    Ok((identity, source))
}
