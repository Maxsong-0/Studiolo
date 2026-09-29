//! canonical 入站源先按冻结证据持久保全，再进入 Broker；未知动作重放只读取保全源。
//! inline／stream 的分界与首次调用完全相同，路径、前置与原幂等键均不改写。

use super::{BridgePrecondition, FRAME_BYTES, INLINE_MAX_BYTES, MirrorWriteError, RoundContext};
use crate::CanonicalWorkbench;
use crate::sync_plane::driver::evidence::hex;
use crate::sync_plane::mirror::PlaneContent;
use crate::sync_plane::mirror::stage::{OBJECTS_DIR, StageStore};
use crate::sync_plane::{PlaneError, quiescence};
use studiolo_core::VaultPath;
use studiolo_sync_bridge::executor::{MirrorContent, ObjectId};
use studiolo_sync_bridge::{FileVersion, Side};
use studiolo_vault_broker::{BlobIdentity, BlobSource, MutationReceipt};

/// 首次写入：票据有效时读完冻结版本，封口并持久登记引用后才可调用 Broker。
///
/// # Errors
/// 静止票据、冻结源或保全失败均为 `Source`（本次尚未调用 Broker）。
pub fn write_from_mirror(
    context: &RoundContext<'_>,
    path: &VaultPath,
    version: &FileVersion,
    precondition: BridgePrecondition,
    key: &str,
    plan_sequence: u64,
) -> Result<MutationReceipt, MirrorWriteError> {
    let frozen = (|| {
        quiescence::verify(context.plane, context.ticket)?;
        let mut content = PlaneContent::new(context.plane, &context.view);
        freeze_source(
            &mut context.stage.borrow_mut(),
            &mut content,
            path,
            version,
            plan_sequence,
            context.now_unix,
        )?;
        quiescence::verify(context.plane, context.ticket)
    })();
    frozen.map_err(|_| MirrorWriteError::Source)?;
    replay_from_stage(context, path, version, precondition, key)
}

/// 重开核对只读既有保全对象；无源或损坏时不访问镜像、不补造原请求。
///
/// # Errors
/// 保全源不可验证返回 `Source`，调用方在重放语境必须保持 Unknown。
pub fn replay_from_stage(
    context: &RoundContext<'_>,
    path: &VaultPath,
    version: &FileVersion,
    precondition: BridgePrecondition,
    key: &str,
) -> Result<MutationReceipt, MirrorWriteError> {
    write_preserved(
        context.plane.workbench(),
        &context.stage.borrow(),
        path,
        version,
        precondition,
        key,
    )
}

/// 逐帧读冻结镜像版本并登记引用；调用方负责在读取前后复验静止票据。
///
/// # Errors
/// 读取、字节数、摘要、stage 预算或索引持久化失败均拒绝后续写入。
pub fn freeze_source(
    stage: &mut StageStore,
    content: &mut dyn MirrorContent,
    path: &VaultPath,
    version: &FileVersion,
    plan_sequence: u64,
    now_unix: i64,
) -> Result<(), PlaneError> {
    content
        .open(Side::Mirror, path, version)
        .map_err(|_| PlaneError::Observation)?;
    stage.begin()?;
    let frozen = (|| {
        let mut read = 0_u64;
        let mut buffer = [0u8; FRAME_BYTES];
        loop {
            let count = content
                .next_frame(&mut buffer)
                .map_err(|_| PlaneError::Observation)?;
            if count == 0 {
                break;
            }
            read = read
                .checked_add(count as u64)
                .ok_or(PlaneError::Observation)?;
            if read > version.bytes {
                return Err(PlaneError::Observation);
            }
            stage.accept(buffer.get(..count).ok_or(PlaneError::Observation)?)?;
        }
        if read != version.bytes {
            return Err(PlaneError::Observation);
        }
        let id = ObjectId::from_digest(version.sha256);
        stage.finish(id)?;
        stage.preserve(id, plan_sequence, path.as_str(), now_unix)
    })();
    if frozen.is_err() {
        let _ = stage.discard();
    }
    frozen
}

/// 用保全源重建唯一请求：小对象整篇写，大对象始终从外 stage 流式写。
/// 首次与重放共用此入口；不读取当前 canonical 或镜像来推断是否已经安装。
///
/// # Errors
/// 源不存在或损坏为 `Source`；进入接缝后的失败保持原分类。
pub fn write_preserved(
    workbench: &CanonicalWorkbench,
    stage: &StageStore,
    path: &VaultPath,
    version: &FileVersion,
    precondition: BridgePrecondition,
    key: &str,
) -> Result<MutationReceipt, MirrorWriteError> {
    let id = ObjectId::from_digest(version.sha256);
    if version.bytes <= INLINE_MAX_BYTES {
        let bytes = stage
            .read_preserved(id, version.bytes, INLINE_MAX_BYTES as usize)
            .map_err(|_| MirrorWriteError::Source)?;
        return workbench
            .bridge_write(path, bytes, precondition, key)
            .map_err(MirrorWriteError::Seam);
    }
    stage
        .verify_preserved(id, version.bytes)
        .map_err(|_| MirrorWriteError::Source)?;
    let object = stage.object_path(id);
    let root = object
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or(MirrorWriteError::Source)?;
    let logical = VaultPath::parse(format!("{OBJECTS_DIR}/{}", hex(&version.sha256)))
        .map_err(|_| MirrorWriteError::Source)?;
    let source = BlobSource::open(root, logical).map_err(|_| MirrorWriteError::Source)?;
    let identity = BlobIdentity {
        bytes: version.bytes,
        sha256: hex(&version.sha256),
    };
    workbench
        .bridge_write_stream(path, source, identity, precondition, key)
        .map_err(MirrorWriteError::Seam)
}
