//! 显式准备已批准的canonical空目录；只通过持有独占租约的Broker写入。
use super::{MaintenanceError, inputs::Inputs, options::Options, sync, workflow::guard};
use crate::sync_plane::driver::evidence::read_selection;
use serde_json::{Value, json};
use studiolo_host_config::SyncScope;
use studiolo_vault_broker::SyncReadSelection;

pub(super) fn selection(scope: &SyncScope) -> Result<SyncReadSelection, MaintenanceError> {
    let selection = scope
        .observation_selection(scope.binding())
        .map_err(|_| MaintenanceError::Configuration)?;
    Ok(read_selection(&selection))
}

pub(super) fn run(
    inputs: &mut Inputs,
    options: &Options,
    confirm: &mut impl FnMut(&Value, &str) -> Result<(), MaintenanceError>,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<Value, MaintenanceError> {
    guard(inputs, check)?;
    let scope = sync::scope(inputs)?;
    let approved = sync::approve(&scope, inputs, options)?;
    let selection = selection(&scope)?;
    let missing = inputs
        .broker
        .inspect_sync_anchors(&selection)
        .map_err(|_| MaintenanceError::SyncAnchors)?;
    let before = inputs
        .broker
        .write_status()
        .map_err(|_| MaintenanceError::Broker)?;
    guard(inputs, check)?;
    confirm(
        &json!({"command":"sync-anchors", "missing_anchors":missing,
        "note":"仅由Broker建立已批准但缺失的空目录；不创建占位文件，不覆盖内容，不改同步范围或传输状态。"}),
        "sync-anchors",
    )?;
    guard(inputs, check)?;
    let checked = sync::approve(&scope, inputs, options)?;
    if checked.approval_digest() != approved.approval_digest() {
        return Err(MaintenanceError::Configuration);
    }
    approved
        .revalidate()
        .map_err(|_| MaintenanceError::Configuration)?;
    if inputs
        .broker
        .inspect_sync_anchors(&selection)
        .map_err(|_| MaintenanceError::SyncAnchors)?
        != missing
    {
        return Err(MaintenanceError::Configuration);
    }
    let created = inputs
        .broker
        .prepare_sync_anchors(&selection)
        .map_err(|_| MaintenanceError::SyncAnchors)?;
    guard(inputs, check).map_err(|_| MaintenanceError::SyncAnchors)?;
    approved
        .revalidate()
        .map_err(|_| MaintenanceError::SyncAnchors)?;
    let after = inputs
        .broker
        .write_status()
        .map_err(|_| MaintenanceError::SyncAnchors)?;
    if before.committed_through != after.committed_through || after.unresolved || after.read_blocked
    {
        return Err(MaintenanceError::SyncAnchors);
    }
    Ok(json!({"command":"sync-anchors", "created_anchors":created,
        "committed_through":after.committed_through,
        "note":"仅准备了范围内目录；没有文件内容变更或新文件revision。正式Bridge导入仍须另验。"}))
}
