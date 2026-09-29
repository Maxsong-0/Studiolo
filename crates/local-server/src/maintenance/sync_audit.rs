//! `sync-purge-residue` 的权威审计：删除前追加 Intent、删除后追加 Outcome，动作名固定
//! `sync.purge-residue`，同一事件标识串起两条。载荷只有计数与文件夹名（`schema_version` 1），
//! Outcome 多一个 `applied`（删除趟是否整趟成功）。事件标识取 32 字节随机数的小写十六进制，
//! 事件与结果先落 plane-state；中断后仅补审计、不重放删除。未知删除结果用 schema 2 的 null 明示。
//! 与维护审计同形；追加后核对回执内容与完整性，任何一处不符都按审计未确认处理。本文件没有 IO 记号。

use super::{MaintenanceError, sync::Counts};
use crate::sync_plane::recovery::{PendingPurge, PlaneStateStore, PurgeResult};
use serde_json::{Value, json};
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditLedger, AuditPhase, SanitizedPayload};

/// 固定动作名。
const ACTION: &str = "sync.purge-residue";

/// 确认后先持久事件号；没有可恢复日记时绝不发 Intent 或删除。
pub(super) fn run(
    ledger: &dyn AuditLedger,
    store: &mut PlaneStateStore,
    folder: &str,
    planned: &Counts,
    delete: impl FnOnce(&mut Counts) -> Result<(), MaintenanceError>,
) -> Result<Counts, MaintenanceError> {
    if store.state().pending_purge.is_some() {
        return Err(MaintenanceError::SyncStorage);
    }
    let mut pending = PendingPurge {
        event_id: event_id()?.to_string(),
        folder: folder.to_owned(),
        planned_files: planned.files as u64,
        planned_directories: planned.directories as u64,
        deletion_started: false,
        result: None,
    };
    save(store, Some(pending.clone()))?;
    intent(ledger, &pending)?;
    pending.deletion_started = true;
    save(store, Some(pending.clone()))?;
    let mut removed = Counts::default();
    // 包括根复验在内的任何删除前失败，也必须先持久并闭合 Outcome。
    let result = delete(&mut removed);
    pending.result = Some(PurgeResult {
        files: Some(removed.files as u64),
        directories: Some(removed.directories as u64),
        applied: Some(result.is_ok()),
    });
    save(store, Some(pending.clone()))?;
    outcome(ledger, &pending)?;
    save(store, None)?;
    result.map(|()| removed)
}

/// 重开只补同一事件的审计，绝不重放删除；删除停点之后中断则结果未知。
/// 返回恢复报告，让用户核对残留后显式再次发起清除。
pub(super) fn recover(
    ledger: &dyn AuditLedger,
    store: &mut PlaneStateStore,
) -> Result<Option<Value>, MaintenanceError> {
    let Some(mut pending) = store.state().pending_purge.clone() else {
        return Ok(None);
    };
    intent(ledger, &pending)?;
    if pending.result.is_none() {
        pending.result = Some(if pending.deletion_started {
            PurgeResult {
                files: None,
                directories: None,
                applied: None,
            }
        } else {
            PurgeResult {
                files: Some(0),
                directories: Some(0),
                applied: Some(false),
            }
        });
        save(store, Some(pending.clone()))?;
    }
    outcome(ledger, &pending)?;
    let result = pending
        .result
        .as_ref()
        .ok_or(MaintenanceError::SyncStorage)?;
    let report = json!({"command":"sync-purge-residue", "recovered_audit":true,
        "folder":pending.folder, "removed_files":result.files, "removed_directories":result.directories,
        "applied":result.applied, "outcome_known":result.applied.is_some(),
        "note":"本次只补齐原清除事件的审计，没有重放删除；请核对镜像残留后再次显式清除。"});
    save(store, None)?;
    Ok(Some(report))
}

fn save(
    store: &mut PlaneStateStore,
    pending: Option<PendingPurge>,
) -> Result<(), MaintenanceError> {
    let mut state = store.state().clone();
    state.pending_purge = pending;
    store.save(state).map_err(|_| MaintenanceError::SyncStorage)
}

fn intent(ledger: &dyn AuditLedger, pending: &PendingPurge) -> Result<(), MaintenanceError> {
    let payload = json!({"schema_version":1,"folder":pending.folder,
        "files":pending.planned_files,"directories":pending.planned_directories});
    let id = AuditEventId::parse(pending.event_id.clone()).map_err(|_| MaintenanceError::Audit)?;
    append(ledger, &id, AuditPhase::Intent, payload)
}

fn outcome(ledger: &dyn AuditLedger, pending: &PendingPurge) -> Result<(), MaintenanceError> {
    let result = pending
        .result
        .as_ref()
        .ok_or(MaintenanceError::SyncStorage)?;
    // 已知结果沿用 schema 1；schema 2 的 null 明确表示崩溃后的未知，不声称删除数为零。
    let payload = json!({"schema_version":if result.applied.is_some() { 1 } else { 2 },
        "folder":pending.folder, "files":result.files,"directories":result.directories,"applied":result.applied});
    let id = AuditEventId::parse(pending.event_id.clone()).map_err(|_| MaintenanceError::Audit)?;
    append(ledger, &id, AuditPhase::Outcome, payload)
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

#[cfg(test)]
#[path = "../../tests/maintenance/sync_purge_recovery.rs"]
mod tests;
