//! 固定 action `sync.transport.run` 的两 phase 映射与真实账本确认。
//!
//! payload 只放封闭字段摘要：没有 stdout/stderr、token、密码、原配置或任意 JSON。

use serde_json::{Value, json};
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditLedger, AuditPhase, AuditRecord, SanitizedPayload,
};
use studiolo_sync_transport::owned_run::{
    AuditPayload, AuditPhaseKind, OutcomeSummary, Quiescence, RequesterSource, RunOperation,
    SyncTransportOwner, TRANSPORT_RUN_ACTION, TransportError,
};

/// 一个活动槽最多两个待补事件；一次冲刷不接受超过这个数量。
const MAX_PENDING: usize = 2;

/// 审计接缝的固定失败分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TransportAuditError {
    /// 权威账本不可用或回执自检失败。
    #[error("权威审计账本不可用")]
    Ledger,
    /// 账本回执与本地期望的事件不一致。
    #[error("审计回执与期望事件不一致")]
    Mismatch,
    /// 请求超过一次冲刷的固定上限。
    #[error("一次冲刷超过固定上限")]
    Budget,
    /// 传输 owner 侧的固定失败。
    #[error("传输运行槽拒绝该操作：{0}")]
    Transport(TransportError),
}

impl From<TransportError> for TransportAuditError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

/// 把封闭候选映射成固定审计事件。
///
/// # Errors
/// 事件标识非法或 payload 被判定含秘密字段时返回 `Mismatch`。
pub fn transport_event(
    event_id: &str,
    phase: AuditPhaseKind,
    payload: &AuditPayload,
) -> Result<AuditEvent, TransportAuditError> {
    let event_id = AuditEventId::parse(event_id).map_err(|_| TransportAuditError::Mismatch)?;
    let payload = SanitizedPayload::try_from(payload_value(payload))
        .map_err(|_| TransportAuditError::Mismatch)?;
    Ok(AuditEvent::new(
        event_id,
        match phase {
            AuditPhaseKind::Intent => AuditPhase::Intent,
            AuditPhaseKind::Outcome => AuditPhase::Outcome,
        },
        TRANSPORT_RUN_ACTION,
        payload,
    ))
}

/// 把当前待补事件逐条追加进权威账本，并把已核验回执登记回运行槽。
///
/// 先 append 再登记：若登记失败，同一条事件下次以原键重试，不会产生第二条记录。
///
/// # Errors
/// 账本不可用返回 `Ledger`；回执与期望不符返回 `Mismatch`。
pub fn flush_transport_audit(
    owner: &mut SyncTransportOwner<'_>,
    ledger: &dyn AuditLedger,
    max_records: usize,
) -> Result<usize, TransportAuditError> {
    flush_budget(max_records)?;
    let mut acknowledged = 0;
    while acknowledged < max_records {
        let Some(candidate) = owner.pending_audit()? else {
            break;
        };
        let expected =
            transport_event(candidate.event_id(), candidate.phase(), candidate.payload())?;
        let record = ledger
            .append(expected.clone())
            .map_err(|_| TransportAuditError::Ledger)?;
        confirm(owner, &candidate, &expected, &record)?;
        acknowledged += 1;
    }
    Ok(acknowledged)
}

/// 一次冲刷的固定上限：一个活动槽最多两个待补事件。
///
/// # Errors
/// 超出 `1..=2` 返回 `Budget`。
pub fn flush_budget(max_records: usize) -> Result<(), TransportAuditError> {
    if max_records == 0 || max_records > MAX_PENDING {
        return Err(TransportAuditError::Budget);
    }
    Ok(())
}

/// 回执自检：完整性链自洽、记录里的事件与本地期望完全一致、偏移非零。
///
/// 任意 offset/hash 本身不是认证；这一步只保证「确认的正是刚追加的那条」。
///
/// # Errors
/// 完整性自检失败返回 `Ledger`；事件不一致返回 `Mismatch`。
pub fn verify_receipt(
    expected: &AuditEvent,
    record: &AuditRecord,
) -> Result<(u64, [u8; 32]), TransportAuditError> {
    record
        .verify_integrity()
        .map_err(|_| TransportAuditError::Ledger)?;
    if record.event() != expected {
        return Err(TransportAuditError::Mismatch);
    }
    let hash = hash_bytes(&record.record_hash().to_hex()).ok_or(TransportAuditError::Ledger)?;
    if record.offset() == 0 {
        return Err(TransportAuditError::Ledger);
    }
    Ok((record.offset(), hash))
}

fn confirm(
    owner: &mut SyncTransportOwner<'_>,
    candidate: &studiolo_sync_transport::owned_run::RunAuditCandidate,
    expected: &AuditEvent,
    record: &AuditRecord,
) -> Result<(), TransportAuditError> {
    let (offset, hash) = verify_receipt(expected, record)?;
    owner.confirm_audit(candidate, offset, hash)?;
    Ok(())
}

fn hash_bytes(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut output = [0u8; 32];
    for (index, slot) in output.iter_mut().enumerate() {
        *slot = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(output)
}

fn payload_value(payload: &AuditPayload) -> Value {
    json!({
        "schema_version": payload.schema_version,
        "run": format!("{}", payload.run_id),
        "sequence": payload.sequence,
        "operation": payload.operation.name(),
        // 只有 SetMode 才有模式；InspectConfig／RunOnce 固定为 null，让审计形状稳定。
        "mode": match payload.operation {
            RunOperation::SetMode(mode) => Some(mode.cli_value()),
            RunOperation::InspectConfig | RunOperation::RunOnce => None,
        },
        "binding_digest": hex(payload.binding_digest),
        "runtime_digest": hex(payload.runtime_digest),
        "backend_digest": hex(payload.backend_digest),
        "budget": {
            "deadline_seconds": payload.budget.deadline_seconds,
            "stop_grace_seconds": payload.budget.stop_grace_seconds,
            "stdout_bytes": payload.budget.stdout_bytes,
            "stderr_bytes": payload.budget.stderr_bytes,
            "config_json_bytes": payload.budget.config_json_bytes,
            "memory_bytes": payload.budget.memory_bytes,
            "pids_max": payload.budget.pids_max,
            "cpu_max_percent": payload.budget.cpu_max_percent,
        },
        "requester": match payload.requester {
            RequesterSource::OwnerSession => "owner_session",
        },
        "outcome": payload.outcome.map(outcome_value),
    })
}

fn outcome_value(outcome: OutcomeSummary) -> Value {
    json!({
        "container_created": outcome.container_created,
        "stop_reason": outcome.stop_reason.map(|reason| format!("{reason:?}")),
        // 只报告「是否实际观察到退出」，绝不声称同步成功或已送达远端。
        "exit_observed": outcome.exit_observed,
        "quiescence": match outcome.quiescence {
            Quiescence::Confirmed => "confirmed",
            Quiescence::Unconfirmed => "unconfirmed",
        },
        "stdout_bytes": outcome.counts.stdout_bytes,
        "stderr_bytes": outcome.counts.stderr_bytes,
        "output_over_budget": outcome.counts.output_over_budget,
    })
}

fn hex(bytes: [u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}
