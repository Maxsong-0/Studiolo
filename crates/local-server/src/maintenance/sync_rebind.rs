//! 静止窗口内重绑定传输安全配置；平面、Bridge、stage 只核验既有存储。

use super::{MaintenanceError, inputs::Inputs, options::Options, sync, workflow::guard};
use crate::sync_plane::{
    BRIDGE_DIR, bridge_binding,
    mirror::stage::StageStore,
    recovery::{PLANE_LOCK, PLANE_STATE_FILE, PlaneStateStore},
};
use serde_json::{Value, json};
use studiolo_sync_bridge::sqlite::BridgeState;
use studiolo_sync_transport::owned_run::TransportRebind;

pub(super) fn run(
    inputs: &Inputs,
    options: &Options,
    confirm: &mut impl FnMut(&Value, &str) -> Result<(), MaintenanceError>,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<Value, MaintenanceError> {
    guard(inputs, check)?;
    let scope = sync::scope(inputs)?;
    let approved = sync::approve(&scope, inputs, options)?;
    let plane_root = approved.roots().plane_root();
    // 先完成传输槽全部拒绝项，再打开可能执行 SQLite checkpoint 的 Bridge。
    let transport =
        TransportRebind::open_existing(&approved).map_err(MaintenanceError::SyncRebind)?;
    // open 的普通启动语义允许首建锁/缺省状态；维护先拒绝缺失固定物，绝不据此初始化。
    for name in [PLANE_LOCK, PLANE_STATE_FILE] {
        if !std::fs::symlink_metadata(plane_root.join(name))
            .is_ok_and(|metadata| metadata.file_type().is_file())
        {
            return Err(MaintenanceError::SyncStorage);
        }
    }
    let plane = PlaneStateStore::open(plane_root).map_err(|_| MaintenanceError::SyncStorage)?;
    if plane.state().started_sequence.is_some() || plane.state().pending_purge.is_some() {
        return Err(MaintenanceError::SyncRebindPending);
    }
    let bridge = BridgeState::open_existing(
        &plane_root.join(BRIDGE_DIR),
        bridge_binding(&approved, inputs.roots.vault_identity()),
    )
    .map_err(|_| MaintenanceError::SyncStorage)?;
    ensure_bridge_idle(&bridge)?;
    // 维护专用打开不回收孤儿；未知、缺失或损坏索引不能被当作空 stage。
    let _stage = StageStore::open_existing_for_maintenance(plane_root)
        .map_err(|_| MaintenanceError::SyncStorage)?;
    let review = transport.review();
    guard(inputs, check)?;
    confirm(
        &json!({"command":"sync-rebind", "binding":review,
            "note":"仅更新同一安装的传输安全配置；保留安装身份与序号，废止旧静止事实。不迁移物理镜像，不改变范围，不初始化或清理存储。"}),
        "sync-rebind",
    )?;
    guard(inputs, check)?;
    // 审批文档本身重新加载，确保确认期间更换安全字段不会被内存快照掩盖。
    let checked = sync::approve(&scope, inputs, options)?;
    if checked.approval_digest() != approved.approval_digest() {
        return Err(MaintenanceError::Configuration);
    }
    approved
        .revalidate()
        .map_err(|_| MaintenanceError::Configuration)?;
    ensure_bridge_idle(&bridge)?;
    let _checked_stage = StageStore::open_existing_for_maintenance(plane_root)
        .map_err(|_| MaintenanceError::SyncStorage)?;
    transport
        .revalidate()
        .map_err(MaintenanceError::SyncRebind)?;
    // 从这里起错误属于结果未知：发布后仍复核所有输入，绝不误报“未写入”。
    let committed = transport
        .commit()
        .map_err(|_| MaintenanceError::SyncRebindOutcome)?;
    guard(inputs, check).map_err(|_| MaintenanceError::SyncRebindOutcome)?;
    let after =
        sync::approve(&scope, inputs, options).map_err(|_| MaintenanceError::SyncRebindOutcome)?;
    if after.approval_digest() != approved.approval_digest() {
        return Err(MaintenanceError::SyncRebindOutcome);
    }
    Ok(json!({"command":"sync-rebind", "binding":committed,
        "note":"既有传输槽已原子重绑定；human 重启后仍须核验运行时与 launcher，旧静止票据不得复用。"}))
}

fn ensure_bridge_idle(bridge: &BridgeState) -> Result<(), MaintenanceError> {
    let pending = bridge
        .view()
        .map_err(|_| MaintenanceError::SyncStorage)?
        .pending_sequence;
    let audit = bridge
        .pending_audit()
        .map_err(|_| MaintenanceError::SyncStorage)?;
    if pending.is_some() || audit.is_some() {
        return Err(MaintenanceError::SyncRebindPending);
    }
    Ok(())
}
