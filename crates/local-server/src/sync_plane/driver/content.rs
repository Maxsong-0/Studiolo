//! canonical 侧正文搬运的共用形态：≤ 4 MiB 整篇条件写，更大走只读来源流式写；
//! D5／D6 的 canonical 版本保全也从这里走。本文件不持有 Broker 类型，只经接缝。

use super::super::mirror::stage::{OBJECTS_DIR, STAGE_DIR, StageStore};
use super::super::mirror::{MirrorView, PlaneContent};
use super::super::witness::seam_error;
use super::super::{PlaneError, SyncPlane, naming};
use super::evidence::{RevisionTable, hex, revision_digest, sha256};
use crate::coordinator::sync_bridge::{BridgePrecondition, BridgeSeamError};
use std::cell::RefCell;
use studiolo_core::VaultPath;
use studiolo_sync_bridge::executor::{MirrorContent, ObjectId};
use studiolo_sync_bridge::{Binding, FileVersion, Side};
use studiolo_sync_transport::owned_run::SyncMode;
use studiolo_vault_broker::{BlobIdentity, BlobSource, MutationReceipt};

/// 整篇条件写的正文上限；更大的对象走流式写。
pub const INLINE_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// 逐帧读正文的固定帧长（与 MirrorCAS 相同）。
pub const FRAME_BYTES: usize = 64 * 1024;

/// 一轮里各步共享的上下文：平面、静止票据给出的视图、外 stage、`Revision` 表、绑定与模式。
pub struct RoundContext<'a> {
    /// 所属平面。
    pub plane: &'a SyncPlane,
    /// 静止票据证明的视图根。
    pub view: MirrorView,
    /// 本轮打开的外 stage。
    pub stage: &'a RefCell<StageStore>,
    /// 本轮观察到的 `Revision` 表。
    pub revisions: &'a RefCell<RevisionTable>,
    /// Bridge 持久状态的绑定。
    pub binding: Binding,
    /// 本轮 headless 所处模式。
    pub mode: SyncMode,
    /// 本轮时刻（Unix 秒）。
    pub now_unix: i64,
}

/// 经视图逐帧读出镜像上冻结版本的正文并核对摘要与字节数。
///
/// # Errors
/// 视图不可用返回 `Unavailable`；正文与冻结版本不符返回 `Observation`。
pub fn read_mirror_bytes(
    plane: &SyncPlane,
    view: &MirrorView,
    path: &VaultPath,
    version: &FileVersion,
) -> Result<Vec<u8>, PlaneError> {
    let mut content = PlaneContent::new(plane, view);
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
/// 分类同接缝。
pub fn write_from_mirror(
    context: &RoundContext<'_>,
    path: &VaultPath,
    version: &FileVersion,
    precondition: BridgePrecondition,
    key: &str,
) -> Result<MutationReceipt, BridgeSeamError> {
    let workbench = context.plane.workbench();
    if version.bytes <= INLINE_MAX_BYTES {
        let bytes = read_mirror_bytes(context.plane, &context.view, path, version)
            .map_err(|_| BridgeSeamError::Unavailable)?;
        return workbench.bridge_write(path, bytes, precondition, key);
    }
    let source = BlobSource::open(context.view.root(), path.clone())
        .map_err(|_| BridgeSeamError::Unavailable)?;
    let identity = BlobIdentity {
        bytes: version.bytes,
        sha256: hex(&version.sha256),
    };
    workbench.bridge_write_stream(path, source, identity, precondition, key)
}

/// 把 canonical 上的冻结版本保全成范围外副本（D5 `ops/bridge-rollback/`、D6 `ops/bridge-conflicts/`）。
///
/// # Errors
/// 副本命名失败、原文件已变或写者不可用返回对应分类；同键重试直接命中原回执。
pub fn preserve_canonical(
    context: &RoundContext<'_>,
    directory: &str,
    path: &VaultPath,
    version: &FileVersion,
    key: &str,
) -> Result<(), PlaneError> {
    let stamp = naming::utc_minute_stamp(context.now_unix).ok_or(PlaneError::Unavailable)?;
    let copy = naming::copy_path(directory, path, naming::CANONICAL_DEVICE, &stamp)
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
