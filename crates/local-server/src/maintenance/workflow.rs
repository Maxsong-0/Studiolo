//! 每一步都复核同一批准根；审计回执确认和维护动作保持显式顺序。

use super::{
    MaintenanceError, audit, flush,
    inputs::Inputs,
    options::{Command, Options},
    presentation, state,
};
use serde_json::{Value, json};
use studiolo_vault_broker::{
    MaintenanceActor, MaintenanceDecision, MaintenanceReceipt, MaintenanceTargetKind,
};

pub(super) fn execute(
    inputs: &mut Inputs,
    options: &Options,
    actor: MaintenanceActor,
    mut confirm: impl FnMut(&Value, &str) -> Result<(), MaintenanceError>,
    check: impl Fn() -> Result<(), MaintenanceError>,
) -> Result<Value, MaintenanceError> {
    guard(inputs, &check)?;
    if options.command == Command::Inspect {
        let observed = state::observe(inputs, true)?;
        guard(inputs, &check)?;
        return Ok(
            json!({"command":"inspect","operator":actor,"audit_available":inputs.audit.is_some(),"status":observed}),
        );
    }
    if inputs.audit.is_none() {
        return Err(MaintenanceError::Audit);
    }
    if options.command == Command::AuditFlush {
        let observed = state::observe(inputs, false)?;
        guard(inputs, &check)?;
        confirm(&json!({"operator":actor,"status":observed}), "audit-flush")?;
        guard(inputs, &check)?;
        flush::all(inputs, &check)?;
        return report(inputs, options.command, None, &check);
    }
    if options.command == Command::CancelMaintenance {
        let selector = options
            .maintenance
            .as_deref()
            .ok_or(MaintenanceError::Arguments)?;
        let observed = inputs
            .broker
            .maintenance_pending()
            .map_err(|_| MaintenanceError::Broker)?;
        guard(inputs, &check)?;
        let pending = observed
            .as_ref()
            .filter(|pending| {
                pending.maintenance_id == selector
                    && !pending.execution_started
                    && pending.outcome.is_none()
            })
            .ok_or(MaintenanceError::Broker)?;
        let original = inputs
            .broker
            .file_operation_status()
            .map_err(|_| MaintenanceError::Broker)?;
        let explanation = presentation::target(&pending.intent.target, original.as_ref())?;
        guard(inputs, &check)?;
        confirm(
            &json!({"operator":actor,"pending":pending,"explanation":explanation,
                "cancellation":"本命令只取消未执行的维护本身，不执行它原先选择的协调动作，也不取消原请求。"}),
            &format!("cancel-maintenance {selector}"),
        )?;
        guard(inputs, &check)?;
        // 维护自身尚未执行也须先补原typed意图；只读固定outbox，不重新观察unsafe目标。
        if pending.intent.target.file_operation_id.is_some() {
            flush::file_operations(inputs, &check)?;
        }
        // 补记原固定 Intent 后才取消维护本身；不开始它原先选择的协调动作。
        flush::maintenance(inputs, &check)?;
        guard(inputs, &check)?;
        let receipt = inputs
            .broker
            .cancel_maintenance(selector, actor)
            .map_err(|_| MaintenanceError::OutcomeUnknown)?;
        flush::all(inputs, &check)?;
        return report(inputs, options.command, Some(receipt), &check);
    }
    let observed = inputs
        .broker
        .maintenance_inspect(64)
        .map_err(|_| MaintenanceError::Broker)?;
    guard(inputs, &check)?;
    let selector = options
        .request
        .as_deref()
        .ok_or(MaintenanceError::Arguments)?;
    let (decision, kind) = match options.command {
        Command::CancelPrepared => (
            MaintenanceDecision::CancelPrepared,
            MaintenanceTargetKind::Prepared,
        ),
        Command::PreserveInstalling => (
            MaintenanceDecision::PreserveInstalling,
            MaintenanceTargetKind::Installing,
        ),
        Command::CancelRestoreIntent => (
            MaintenanceDecision::CancelRestoreIntent,
            MaintenanceTargetKind::RestoreIntent,
        ),
        _ => return Err(MaintenanceError::Arguments),
    };
    let target = observed
        .targets
        .iter()
        .find(|target| target.review_id == selector && target.kind == kind)
        .ok_or(MaintenanceError::Broker)?;
    let original = inputs
        .broker
        .file_operation_status()
        .map_err(|_| MaintenanceError::Broker)?;
    let explanation = presentation::target(target, original.as_ref())?;
    guard(inputs, &check)?;
    confirm(
        &json!({"operator":actor,"target":target,"explanation":explanation}),
        &format!("{} {selector}", options.command.name()),
    )?;
    guard(inputs, &check)?;
    // 用户确认后才补记原 typed Intent；ACK 不执行原删除/恢复。
    if target.file_operation_id.is_some() {
        flush::file_operations(inputs, &check)?;
    }
    let ticket = inputs
        .broker
        .prepare_maintenance(selector, decision, actor)
        .map_err(|_| MaintenanceError::Broker)?;
    guard(inputs, &check)?;
    let (offset, hash) = audit::append(
        inputs.audit.as_deref().ok_or(MaintenanceError::Audit)?,
        ticket.intent(),
    )?;
    guard(inputs, &check)?;
    inputs
        .broker
        .confirm_maintenance_intent(&ticket, offset, &hash)
        .map_err(|_| MaintenanceError::Audit)?;
    guard(inputs, &check)?;
    let receipt = inputs
        .broker
        .commit_maintenance(ticket)
        .map_err(|_| MaintenanceError::OutcomeUnknown)?;
    flush::all(inputs, &check)?;
    report(inputs, options.command, Some(receipt), &check)
}

pub(super) fn guard(
    inputs: &Inputs,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<(), MaintenanceError> {
    check()?;
    inputs.revalidate()?;
    check()
}

fn report(
    inputs: &Inputs,
    command: Command,
    receipt: Option<MaintenanceReceipt>,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<Value, MaintenanceError> {
    guard(inputs, check)?;
    let status = state::observe(inputs, false).map_err(|_| MaintenanceError::OutcomeUnknown)?;
    let content = inputs
        .broker
        .content_audit_status()
        .map_err(|_| MaintenanceError::OutcomeUnknown)?;
    guard(inputs, check)?;
    let explanation = receipt
        .as_ref()
        .map(|value| presentation::result(&value.event));
    Ok(
        json!({"command":command.name(),"receipt":receipt,"status":status,
        "explanation":explanation,"content_audit_pending":content.pending,
        "restore_unconfirmed":content.restore_unconfirmed}),
    )
}
