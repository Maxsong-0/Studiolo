//! 经传输运行槽跑一次固定操作到闭合，以及 InspectConfig 输出对审批值的核对。
//!
//! 只看阶段、退出码与静止证据，不解析 stderr；审计经 `sync_transport_owner::audit` 走权威账本。

use super::super::{PlaneError, SyncPlane};
use super::HeadlessRun;
use super::evidence::hex;
use crate::sync_transport_config::{ConfigError, Expected, FileType, verify};
use crate::sync_transport_owner::flush_transport_audit;
use std::time::{Duration, Instant};
use studiolo_audit_ledger::AuditLedger;
use studiolo_sync_transport::owned_run::{
    ExitObservation, Quiescence, RunOperation, RunPhase, StopReason, SyncMode, SyncTransportOwner,
    TransportError,
};

/// 两次 `wait` 之间的固定间隔。
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// 一个活动槽最多两个待补事件。
const AUDIT_BATCH: usize = 2;

/// headless `sync-config --json` 报的附件类型必须恰是这五类（ADR-025 D12）。
const FILE_TYPES: [FileType; 5] = [
    FileType::Image,
    FileType::Audio,
    FileType::Video,
    FileType::Pdf,
    FileType::Unsupported,
];

/// 跑一次固定操作到 Closed：先补审计、回收旧槽，再 prepare → Intent ACK → start →
/// 轮询 wait（超 deadline 就 request_stop）→ Outcome ACK → 从闭合摘要取观察。
///
/// # Errors
/// 传输槽拒绝返回 `Transport`；审计不可用返回 `Audit`；槽锁不可用返回 `Unavailable`。
pub fn run(
    plane: &SyncPlane,
    operation: RunOperation,
    now_unix: i64,
) -> Result<HeadlessRun, PlaneError> {
    let mut transport = plane
        .transport()
        .lock()
        .map_err(|_| PlaneError::Unavailable)?;
    let ledger: &dyn AuditLedger = plane.audit().as_ref();
    settle_active(&mut transport, ledger)?;
    let status = transport.inspect().map_err(transport_error)?;
    let prepared = transport
        .prepare_once(status.next_sequence, operation, millis(now_unix))
        .map_err(transport_error)?;
    flush_transport_audit(&mut transport, ledger, AUDIT_BATCH).map_err(|_| PlaneError::Audit)?;
    let owned = transport.start(&prepared).map_err(transport_error)?;
    let deadline = Duration::from_secs(plane.approval().budget().deadline_seconds.into());
    let started = Instant::now();
    let mut stop_requested = false;
    loop {
        let observation = transport.wait(&owned).map_err(transport_error)?;
        if matches!(
            observation.phase,
            RunPhase::OutcomePending | RunPhase::Closed | RunPhase::Cancelled
        ) {
            break;
        }
        if !stop_requested && observation.stop_reason.is_none() && started.elapsed() > deadline {
            transport
                .request_stop(&owned, StopReason::DeadlineExceeded)
                .map_err(transport_error)?;
            stop_requested = true;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    flush_transport_audit(&mut transport, ledger, AUDIT_BATCH).map_err(|_| PlaneError::Audit)?;
    let status = transport.inspect().map_err(transport_error)?;
    if status.active.is_some() {
        return Err(PlaneError::Audit);
    }
    let terminal = status
        .last_terminal
        .filter(|terminal| terminal.run_id == owned.run_id())
        .ok_or(PlaneError::Transport)?;
    if terminal.observation.phase != RunPhase::Closed {
        return Err(PlaneError::Transport);
    }
    Ok(HeadlessRun {
        run_id: terminal.run_id,
        observation: terminal.observation,
        clean: terminal.observation.exit == ExitObservation::Code(0)
            && terminal.observation.quiescence == Quiescence::Confirmed,
        rc_one: terminal.observation.exit == ExitObservation::Code(1),
    })
}

/// 第 2 步：补审计；活动槽存在则只观察旧容器（绝不重新执行），仍未闭合即 `Transport`。
fn settle_active(
    transport: &mut SyncTransportOwner<'_>,
    ledger: &dyn AuditLedger,
) -> Result<(), PlaneError> {
    flush_transport_audit(transport, ledger, AUDIT_BATCH).map_err(|_| PlaneError::Audit)?;
    let Some(active) = transport.inspect().map_err(transport_error)?.active else {
        return Ok(());
    };
    let observation = transport.recover(active.run_id).map_err(transport_error)?;
    if observation.phase == RunPhase::OutcomePending {
        flush_transport_audit(transport, ledger, AUDIT_BATCH).map_err(|_| PlaneError::Audit)?;
    }
    if transport
        .inspect()
        .map_err(transport_error)?
        .active
        .is_some()
    {
        return Err(PlaneError::Transport);
    }
    Ok(())
}

/// 核对 InspectConfig 的 JSON 输出：远端 Vault、镜像根、设备名、`.obsidian`、五类附件、
/// 模式与排除项逐项等于审批值。
///
/// # Errors
/// 任一项不一致返回 `Mismatch`；审批值本身不合法返回 `InvalidExpected`。
pub(crate) fn verify_config(
    plane: &SyncPlane,
    output: &[u8],
    mode: SyncMode,
) -> Result<(), ConfigError> {
    let approval = plane.approval();
    let remote_vault_id = hex(&approval.binding().remote_vault);
    let mirror_path = approval
        .roots()
        .mirror_root()
        .to_str()
        .ok_or(ConfigError::InvalidExpected)?;
    verify(
        output,
        &Expected {
            remote_vault_id: &remote_vault_id,
            mirror_path,
            device_name: approval.device_name(),
            config_dir: ".obsidian",
            file_types: &FILE_TYPES,
            sync_mode: mode.cli_value(),
            excluded_folders: approval.excluded_folders(),
        },
    )
    .map(|_| ())
}

/// 传输层失败到平面失败：审计类归 `Audit`，其余一律 `Transport`。
pub fn transport_error(error: TransportError) -> PlaneError {
    match error {
        TransportError::AuditPending
        | TransportError::AuditMismatch
        | TransportError::AuditRejected => PlaneError::Audit,
        _ => PlaneError::Transport,
    }
}

fn millis(now_unix: i64) -> u64 {
    u64::try_from(now_unix).unwrap_or(0).saturating_mul(1000)
}
