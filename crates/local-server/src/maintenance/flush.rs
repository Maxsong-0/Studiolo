//! 固定审计分层补记；原 typed Outcome 必须先于维护 Outcome 获得真实 ACK。

use super::{MaintenanceError, audit, inputs::Inputs, workflow::guard};

pub(super) fn all(
    inputs: &mut Inputs,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<(), MaintenanceError> {
    guard(inputs, check)?;
    // 保留原 Human/Obsidian actor 与请求种类；本机操作者不会冒充原调用者。
    // 同一原请求每轮只出一个阶段；最多两轮补齐 Intent 与 Outcome，批次始终有界。
    for _ in 0..2 {
        guard(inputs, check)?;
        let candidates = inputs
            .broker
            .pending_content_audit(64)
            .map_err(|_| MaintenanceError::Audit)?;
        if candidates.is_empty() {
            break;
        }
        for candidate in candidates {
            guard(inputs, check)?;
            let (offset, hash) = crate::editing::audit::append(
                inputs.audit.as_deref().ok_or(MaintenanceError::Audit)?,
                candidate.event(),
            )
            .map_err(|_| MaintenanceError::Audit)?;
            guard(inputs, check)?;
            inputs
                .broker
                .confirm_content_audit(candidate, offset, &hash)
                .map_err(|_| MaintenanceError::Audit)?;
        }
    }
    file_operations(inputs, check)?;
    maintenance(inputs, check)
}

pub(super) fn maintenance(
    inputs: &mut Inputs,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<(), MaintenanceError> {
    guard(inputs, check)?;
    let candidates = inputs
        .broker
        .pending_maintenance_audit(64)
        .map_err(|_| MaintenanceError::Audit)?;
    for candidate in candidates {
        guard(inputs, check)?;
        let (offset, hash) = audit::append(
            inputs.audit.as_deref().ok_or(MaintenanceError::Audit)?,
            candidate.event(),
        )?;
        guard(inputs, check)?;
        inputs
            .broker
            .confirm_maintenance_audit(candidate, offset, &hash)
            .map_err(|_| MaintenanceError::Audit)?;
    }
    guard(inputs, check)
}

pub(super) fn file_operations(
    inputs: &mut Inputs,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<(), MaintenanceError> {
    // Intent ACK 后可能固定 Rejected Outcome，因此需要第二轮；单活动槽最多两个事件。
    for _ in 0..2 {
        guard(inputs, check)?;
        let candidates = inputs
            .broker
            .pending_file_operation_audit(2)
            .map_err(|_| MaintenanceError::Audit)?;
        if candidates.is_empty() {
            break;
        }
        for candidate in candidates {
            guard(inputs, check)?;
            let (offset, hash) = crate::file_operation_audit::append(
                inputs.audit.as_deref().ok_or(MaintenanceError::Audit)?,
                candidate.event(),
            )
            .map_err(|_| MaintenanceError::Audit)?;
            guard(inputs, check)?;
            inputs
                .broker
                .confirm_file_operation_audit(candidate, offset, &hash)
                .map_err(|_| MaintenanceError::Audit)?;
        }
    }
    guard(inputs, check)
}
