//! `sync-purge-residue` 的权威审计：删除前追加 Intent、删除后追加 Outcome，动作名固定
//! `sync.purge-residue`，同一事件标识串起两条。载荷只有计数与文件夹名（`schema_version` 1），
//! Outcome 多一个 `applied`（删除趟是否整趟成功）。事件标识取 32 字节随机数的小写十六进制，
//! 与维护审计同形；追加后核对回执内容与完整性，任何一处不符都按审计未确认处理。本文件没有 IO 记号。

use super::{MaintenanceError, sync::Counts};
use serde_json::{Value, json};
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditLedger, AuditPhase, SanitizedPayload};

/// 固定动作名。
const ACTION: &str = "sync.purge-residue";

/// 删除前的 Intent：将要删除的文件与目录数（核验趟的计数）。
pub(super) fn intent(
    ledger: &dyn AuditLedger,
    folder: &str,
    planned: &Counts,
) -> Result<AuditEventId, MaintenanceError> {
    let id = event_id()?;
    let payload = json!({"schema_version":1,"folder":folder,
        "files":planned.files,"directories":planned.directories});
    append(ledger, &id, AuditPhase::Intent, payload)?;
    Ok(id)
}

/// 删除后的 Outcome：实际删掉的文件与目录数，以及删除趟是否整趟成功。
pub(super) fn outcome(
    ledger: &dyn AuditLedger,
    id: &AuditEventId,
    folder: &str,
    removed: &Counts,
    applied: bool,
) -> Result<(), MaintenanceError> {
    let payload = json!({"schema_version":1,"folder":folder,
        "files":removed.files,"directories":removed.directories,"applied":applied});
    append(ledger, id, AuditPhase::Outcome, payload)
}

fn append(
    ledger: &dyn AuditLedger,
    id: &AuditEventId,
    phase: AuditPhase,
    payload: Value,
) -> Result<(), MaintenanceError> {
    let payload = SanitizedPayload::try_from(payload).map_err(|_| MaintenanceError::Audit)?;
    let event = AuditEvent::new(id.clone(), phase, ACTION, payload);
    let record = ledger
        .append(event.clone())
        .map_err(|_| MaintenanceError::Audit)?;
    if record.event() != &event || record.verify_integrity().is_err() {
        return Err(MaintenanceError::Audit);
    }
    Ok(())
}

/// 32 字节随机数的小写十六进制；随机源不可用即审计不可用，不删任何东西。
fn event_id() -> Result<AuditEventId, MaintenanceError> {
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| MaintenanceError::Audit)?;
    let hex: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    AuditEventId::parse(hex).map_err(|_| MaintenanceError::Audit)
}
