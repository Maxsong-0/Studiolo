//! 固定维护槽与原目标新审阅分开；原目标不可读不能阻止维护自身收口。

use super::{MaintenanceError, inputs::Inputs, presentation};
use serde_json::{Value, json};

pub(super) fn observe(inputs: &Inputs, inspect_targets: bool) -> Result<Value, MaintenanceError> {
    inputs.revalidate()?;
    let pending = inputs
        .broker
        .maintenance_pending()
        .map_err(|_| MaintenanceError::Broker)?;
    let writer = inputs
        .broker
        .write_status()
        .map_err(|_| MaintenanceError::Broker)?;
    let operation = inputs
        .broker
        .file_operation_status()
        .map_err(|_| MaintenanceError::Broker)?;
    let mut value = json!({"writer":writer,"pending":pending,"targets":null,
        "targets_status":"not_observed","diagnostics":[]});
    if let Some(operation) = &operation {
        value["file_operation"] =
            serde_json::to_value(operation).map_err(|_| MaintenanceError::Output)?;
        value["file_operation_note"] = presentation::original(operation);
    }
    if inspect_targets {
        match inputs.broker.maintenance_inspect(64) {
            Ok(full) => {
                if full.writer != writer || full.pending != pending {
                    return Err(MaintenanceError::Broker);
                }
                value["target_notes"] = full
                    .targets
                    .iter()
                    .map(|target| presentation::target(target, operation.as_ref()))
                    .collect::<Result<Vec<_>, _>>()?
                    .into();
                value["targets"] =
                    serde_json::to_value(full.targets).map_err(|_| MaintenanceError::Output)?;
                value["targets_status"] = "ready".into();
            }
            Err(_) => {
                value["targets_status"] = "unavailable".into();
                value["diagnostics"] =
                    json!(["原目标不能完整重新审阅；这里只发布已验证的固定维护状态"]);
            }
        }
    }
    // 完整审阅失败也必须再次验证固定槽；不能掩盖其损坏、替换或中途变化。
    if inputs
        .broker
        .maintenance_pending()
        .map_err(|_| MaintenanceError::Broker)?
        != pending
        || inputs
            .broker
            .write_status()
            .map_err(|_| MaintenanceError::Broker)?
            != writer
        || inputs
            .broker
            .file_operation_status()
            .map_err(|_| MaintenanceError::Broker)?
            != operation
    {
        return Err(MaintenanceError::Broker);
    }
    inputs.revalidate()?;
    Ok(value)
}
