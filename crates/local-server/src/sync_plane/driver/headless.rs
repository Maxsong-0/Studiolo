//! 经传输运行槽跑一次固定操作到闭合，以及 InspectConfig 输出对审批值的核对。
//!
//! 只看阶段、退出码与静止证据，不解析 stderr；审计经 `sync_transport_owner::audit` 走权威账本。

use super::super::{PlaneError, SyncPlane};
use super::evidence::hex;
use super::{HeadlessRun, Inspection};
use crate::sync_transport_config::{ConfigError, Expected, FileType, verify};
use crate::sync_transport_owner::flush_transport_audit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use studiolo_audit_ledger::AuditLedger;
use studiolo_sync_transport::owned_run::{
    ExitObservation, Quiescence, RunId, RunObservation, RunOperation, RunPhase, StopReason,
    SyncMode, SyncTransportOwner, TransportError,
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
/// 轮询 wait（关停已请求就 request_stop(OwnerRequested)，超 deadline 就 request_stop(DeadlineExceeded)）
/// → Outcome ACK → 从闭合摘要取观察。被停下的运行照样轮询到闭合、干净回收后才返回。
///
/// # Errors
/// 传输槽拒绝返回 `Transport`；审计不可用返回 `Audit`；槽锁不可用返回 `Unavailable`。
pub fn run(
    plane: &SyncPlane,
    operation: RunOperation,
    now_unix: i64,
    stopping: &AtomicBool,
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
        if !stop_requested && observation.stop_reason.is_none() {
            // 关停优先于时限：两者同时成立时按 owner 关停记，停止原因第一次持久即最终原因。
            let reason = if stopping.load(Ordering::Acquire) {
                Some(StopReason::OwnerRequested)
            } else if started.elapsed() > deadline {
                Some(StopReason::DeadlineExceeded)
            } else {
                None
            };
            if let Some(reason) = reason {
                transport
                    .request_stop(&owned, reason)
                    .map_err(transport_error)?;
                stop_requested = true;
            }
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
    Ok(HeadlessRun::from_terminal(
        terminal.run_id,
        terminal.observation,
    ))
}

impl HeadlessRun {
    /// 从闭合摘要分类：退出码 0、静止已证明、**且没有被停止**才算干净。headless 收到 SIGTERM
    /// 会以 rc=0 退出（P2 S9a/b），被 owner 关停、超时或输出超限停下的运行都说明操作被打断，
    /// 必须记成非干净，下一轮走 D9 强制恢复（评审 2026-09-28 P2-1）。
    #[must_use]
    pub fn from_terminal(run_id: RunId, observation: RunObservation) -> Self {
        Self {
            run_id,
            observation,
            clean: observation.exit == ExitObservation::Code(0)
                && observation.quiescence == Quiescence::Confirmed
                && observation.stop_reason.is_none(),
            rc_one: observation.exit == ExitObservation::Code(1),
        }
    }

    /// 非 clean 运行的失败分类：owner 关停时被停止即 `Stopping`，其余为 `Headless`，保留闭合退出证据。
    #[must_use]
    pub fn failure(&self) -> PlaneError {
        if self.observation.stop_reason == Some(StopReason::OwnerRequested) {
            PlaneError::Stopping
        } else {
            PlaneError::Headless
        }
    }
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

/// 第 3 步的核对顺序（真实端口与契约测试的假端口共用这一份）。`matches(排除集合, 模式)` 回答
/// 「headless 输出是否恰等于这组审批值」；依次试 ① 审批母本＋应处模式 → ② 母本＋另一模式（只差模式，
/// 交第 5 步 SetMode）→ ③ `pending_purge` 非空时的并集（母本 ∪ 待清文件夹）＋应处模式 → ④ 并集＋另一模式，
/// 命中哪份母本就把它记成 `headless_excluded`。`pending_purge`＝上次核验集合 − 审批母本：
/// 审批已去掉、headless 仍排除、等清残留的文件夹（排除项安全顺序进行中）。
///
/// # Errors
/// 四种组合都不一致返回 `ConfigMismatch`。
pub fn classify(
    expected: SyncMode,
    approved: &[String],
    verified: &[String],
    matches: &mut dyn FnMut(&[String], SyncMode) -> bool,
) -> Result<Inspection, PlaneError> {
    let other = match expected {
        SyncMode::MirrorRemote => SyncMode::Bidirectional,
        SyncMode::Bidirectional => SyncMode::MirrorRemote,
    };
    let mut pending: Vec<String> = verified
        .iter()
        .filter(|folder| !approved.contains(folder))
        .cloned()
        .collect();
    pending.sort();
    pending.dedup();
    let mut superset: Vec<String> = approved.iter().chain(&pending).cloned().collect();
    superset.sort();
    superset.dedup();
    let mut candidates = vec![(approved.to_vec(), Vec::new())];
    if !pending.is_empty() {
        candidates.push((superset, pending));
    }
    for (headless_excluded, pending_purge) in candidates {
        for (mode, mode_matches) in [(expected, true), (other, false)] {
            if matches(&headless_excluded, mode) {
                return Ok(Inspection {
                    mode_matches,
                    excluded_folders: approved.to_vec(),
                    headless_excluded,
                    pending_purge,
                });
            }
        }
    }
    Err(PlaneError::ConfigMismatch)
}

/// 核对 InspectConfig 的 JSON 输出：远端 Vault、镜像根、设备名、`.obsidian`、五类附件、
/// 模式与排除项逐项等于给定值；`excluded` 必须已排序（审批母本或 `classify` 算出的并集）。
///
/// # Errors
/// 任一项不一致返回 `Mismatch`；审批值本身不合法返回 `InvalidExpected`。
pub(crate) fn verify_config(
    plane: &SyncPlane,
    output: &[u8],
    mode: SyncMode,
    excluded: &[String],
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
            excluded_folders: excluded,
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
