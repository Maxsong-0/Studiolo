//! 平面的健康证据出口：第六节 `BridgeEvidence`、owner 活性、Sync 配额读数。
//!
//! 本文件只累积事实，不读磁盘；`startup/health_input.rs` 经 `BridgeHealthProbe` 取快照。
//! 每个回填方法只写它能证明的字段：驱动器在轮内调 `record_mode`／`record_headless`／
//! `record_observed_copies`／`record_conflicts`／`record_execution`／`record_usage`，轮末由
//! `SyncPlane::drive_until` 调 `record_round` 收口；没被任何方法证明过的字段保持 `None`，
//! 第六节因此判未知而不是判正常。

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use studiolo_host_config::sync_transport::ManualQuota;
use studiolo_observability::health::{
    BatchStamp, BridgeEvidence, ExitClass, Instant, ModeLabel, OwnerLabel, OwnerLiveness,
    RecoveryState, ResidueGuard,
};
use studiolo_observability::quota::SyncUsage;
use studiolo_sync_transport::owned_run::{ExitObservation, StopReason, SyncMode};

use super::PlaneError;
use super::driver::{HeadlessRun, RoundOutcome};

/// 健康页上传输 owner 的角色标签。
pub const OWNER_LABEL: &str = "sync-transport";

/// 装配层读健康证据的唯一入口；只读，实现不得借此触发搬运。
pub trait BridgeHealthProbe: Send + Sync {
    /// 第六节证据；拿不到的字段一律 `None`。
    fn bridge(&self) -> BridgeEvidence;
    /// 传输 owner 的活性回执（最近一轮时刻，约定周期 = 两倍轮间隔）。
    fn liveness(&self) -> OwnerLiveness;
    /// Studiolo 侧镜像字节数与对象数；不是账号用量。
    fn sync_usage(&self) -> Option<SyncUsage>;
    /// 审批文档里的人工配额；`None` 即该维度未知。
    fn manual_quota(&self) -> Option<ManualQuota>;
}

/// 平面内的健康累积器。
pub struct PlaneHealth {
    manual_quota: Option<ManualQuota>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    last_round_unix: Option<i64>,
    evidence: BridgeEvidence,
    usage: Option<SyncUsage>,
    /// 最近一次 `record_execution` 报的计划序号；轮末据此判断本轮有没有执行段回填。
    executed_sequence: Option<u64>,
    /// 持久 `conflict_stamps` 中当前待解决的冲突数；包含已配戳但副本尚在续写的冲突。
    conflicts_total: u64,
    /// 最近一次完整观察里两侧命名为冲突副本的路径数；不累计，每次观察覆盖，重启后下一次观察即恢复。
    observed_copies: u64,
}

impl PlaneHealth {
    /// 以审批文档里的人工配额起步；其余字段在驱动轮次里逐步填入。
    #[must_use]
    pub fn new(manual_quota: Option<ManualQuota>) -> Self {
        Self {
            manual_quota,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// 记录一轮结束，按结论收口第六节证据；时刻同时作为传输 owner 的活性回执。
    ///
    /// 只写本轮结论能证明的字段：`Settled` 证明整轮干净走完，把熔断、终态计数一并复位；
    /// `Idle`／`Unknown`／`Blocked` 只证明跑到了执行段（守卫放行、RunOnce 干净回收、协议空闲）；
    /// 每种 `PlaneError` 只落到它对应的那一个字段，其余保持原值。`quiescence_valid` 按
    /// 「本轮是否取到静止票据」置位：除 `Skipped` 外的 `Ok` 都出现在取票之后，任何 `Err`
    /// 与 `Skipped` 都没有新票据，第六节至少落到未知，失败轮不可能被上一轮的绿盖住。
    pub fn record_round(&self, now_unix: i64, outcome: &Result<RoundOutcome, PlaneError>) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        inner.last_round_unix = Some(now_unix);
        let executed = inner.executed_sequence;
        let evidence = &mut inner.evidence;
        evidence.quiescence_valid = Some(matches!(
            outcome,
            Ok(RoundOutcome::Idle
                | RoundOutcome::Settled { .. }
                | RoundOutcome::Unknown { .. }
                | RoundOutcome::Blocked { .. })
        ));
        match outcome {
            Ok(RoundOutcome::Settled { sequence, .. }) => {
                stamp(evidence, *sequence, now_unix);
                evidence.fused = Some(false);
                evidence.unknown_jobs = Some(0);
            }
            Ok(RoundOutcome::Idle) => reached_execution(evidence),
            Ok(RoundOutcome::Unknown { sequence }) => {
                stamp(evidence, *sequence, now_unix);
                // 执行段报了本批的数就以它为准，没报至少有一条。
                let reported = evidence
                    .unknown_jobs
                    .filter(|_| executed == Some(*sequence));
                evidence.unknown_jobs = Some(reported.unwrap_or(0).max(1));
            }
            Ok(RoundOutcome::Blocked { sequence }) => {
                stamp(evidence, *sequence, now_unix);
                // 冻结原因只有执行段知道；它没报本批就不敢说熔断没生效。
                if executed != Some(*sequence) {
                    evidence.fused = None;
                }
            }
            Ok(RoundOutcome::Skipped) | Err(PlaneError::Backoff) => {
                evidence.recovery = Some(RecoveryState::Backoff);
            }
            // 强制恢复轮干净跑完：恢复协议仍在进行（下一轮才回审批模式），本轮无静止票据。
            Ok(RoundOutcome::Recovered) => {
                evidence.recovery = Some(RecoveryState::ForcedMirrorRemote);
            }
            Err(PlaneError::Fused) => evidence.fused = Some(true),
            Err(PlaneError::ResidueBlocked) => {
                evidence.residue_guard = Some(ResidueGuard::Blocked);
            }
            Err(PlaneError::Transport) => evidence.last_exit = Some(ExitClass::Unconfirmed),
            Err(PlaneError::Unknown) => {
                evidence.unknown_jobs = Some(evidence.unknown_jobs.unwrap_or(0).max(1));
            }
            Err(PlaneError::ConfigMismatch) => evidence.config_matches = Some(false),
            // 关停停点结束的轮次没有专属字段：本轮没取到新票据，静止证据照上面置为过期。
            Err(
                PlaneError::Headless
                | PlaneError::Configuration
                | PlaneError::Quiescence
                | PlaneError::Observation
                | PlaneError::Audit
                | PlaneError::Storage
                | PlaneError::Unavailable
                | PlaneError::Stopping,
            ) => {}
        }
    }

    /// 执行段回填（驱动器在 `reserve_and_execute` 之后调）：本批序号、终态未知的动作数、
    /// 删除熔断是否生效；冲突副本数＝最近一次完整观察里两侧命名为冲突副本的路径数
    /// ＋持久 `conflict_stamps` 中当前待解决的冲突数（饱和加）。
    pub fn record_execution(&self, sequence: u64, unknown: u64, fused: bool) {
        if let Ok(mut inner) = self.inner.lock() {
            let copies = inner.conflicts_total.saturating_add(inner.observed_copies);
            inner.executed_sequence = Some(sequence);
            inner.evidence.unknown_jobs = Some(unknown);
            inner.evidence.conflict_copies = Some(copies);
            inner.evidence.fused = Some(fused);
        }
    }

    /// headless 一次运行回收后回填（驱动器在 `headless::run` 返回后调）。
    ///
    /// 按实际观察分类，不解析 stderr：`clean`（退出码 0 且静止已证明）→ 干净退出；owner 关停时
    /// 被 `request_stop` 停下 → owner 关停时停止；超时限被停 → 超过时限；被信号终止 → 信号；
    /// 非零退出码 → 非零退出；其余（没取得等待结果、退出 0 但静止未证明）→ 退出结果未确认。
    /// `forced_recovery` 为真即本次是 D9 的强制 `mirror-remote` 恢复运行。
    pub fn record_headless(&self, run: &HeadlessRun, forced_recovery: bool) {
        let observation = run.observation;
        let exit = if run.clean {
            ExitClass::Clean
        } else if observation.stop_reason == Some(StopReason::OwnerRequested) {
            ExitClass::OwnerStopped
        } else if observation.stop_reason == Some(StopReason::DeadlineExceeded) {
            ExitClass::DeadlineExceeded
        } else {
            match observation.exit {
                ExitObservation::Signal(_) => ExitClass::Signal,
                ExitObservation::Code(code) if code != 0 => ExitClass::NonZero,
                _ => ExitClass::Unconfirmed,
            }
        };
        if let Ok(mut inner) = self.inner.lock() {
            inner.evidence.last_exit = Some(exit);
            inner.evidence.recovery = Some(if forced_recovery {
                RecoveryState::ForcedMirrorRemote
            } else {
                RecoveryState::Idle
            });
        }
    }

    /// 本轮核验通过的生效模式（驱动器在 InspectConfig 复核通过后每轮调一次）。
    pub fn record_mode(&self, mode: SyncMode) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.evidence.config_matches = Some(true);
            inner.evidence.mode = Some(match mode {
                SyncMode::MirrorRemote => ModeLabel::MirrorRemote,
                SyncMode::Bidirectional => ModeLabel::Bidirectional,
            });
        }
    }

    /// 覆盖为持久冲突戳的当前条数：启动、配戳与回收后都从同一份状态回填。
    /// 该值代表待解决冲突，既不冒称副本均已写完，也不代表 `ops/` 的历史文件总数。
    pub fn record_conflicts(&self, count: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.conflicts_total = count;
            inner.evidence.conflict_copies = Some(count.saturating_add(inner.observed_copies));
        }
    }

    /// 最近一次完整观察里两侧命名为冲突副本的路径数（headless 设备名与 `studiolo-canonical`
    /// 都识别，见 `driver::observe::conflict_copy_count`）；每次观察覆盖，不累计。
    pub fn record_observed_copies(&self, count: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.observed_copies = count;
            inner.evidence.conflict_copies = Some(count.saturating_add(inner.conflicts_total));
        }
    }

    /// 持久状态中当前待解决的冲突数。
    #[must_use]
    pub fn conflicts_total(&self) -> u64 {
        self.inner
            .lock()
            .map(|inner| inner.conflicts_total)
            .unwrap_or(0)
    }

    /// 驱动器在观察镜像后回填的字节数与对象数。
    pub fn record_usage(&self, usage: SyncUsage) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.usage = Some(usage);
            inner.evidence.mirror_bytes = Some(usage.bytes);
        }
    }
}

/// 一批闭合或停住：盖上序号与时刻，并记下跑到执行段所证明的事实。
fn stamp(evidence: &mut BridgeEvidence, sequence: u64, now_unix: i64) {
    evidence.last_batch = Some(BatchStamp {
        sequence,
        unix_seconds: now_unix,
    });
    reached_execution(evidence);
}

/// 跑到执行段所证明的事实：守卫放行（第 4 步）、RunOnce 干净回收（第 6～7 步）、
/// 恢复协议不再进行中（强制恢复运行一旦干净回收就回到审批模式）。
fn reached_execution(evidence: &mut BridgeEvidence) {
    evidence.residue_guard = Some(ResidueGuard::Clear);
    evidence.last_exit = Some(ExitClass::Clean);
    evidence.recovery = Some(RecoveryState::Idle);
}

impl BridgeHealthProbe for PlaneHealth {
    fn bridge(&self) -> BridgeEvidence {
        self.inner
            .lock()
            .map(|inner| inner.evidence)
            .unwrap_or_default()
    }

    fn liveness(&self) -> OwnerLiveness {
        let last = self
            .inner
            .lock()
            .ok()
            .and_then(|inner| inner.last_round_unix);
        OwnerLiveness {
            owner: OwnerLabel::parse(OWNER_LABEL).expect("固定标签合法"),
            last_receipt: last.map(Instant::from_unix_seconds),
            expected_period_seconds: super::ROUND_INTERVAL.as_secs().saturating_mul(2),
        }
    }

    fn sync_usage(&self) -> Option<SyncUsage> {
        self.inner.lock().ok().and_then(|inner| inner.usage)
    }

    fn manual_quota(&self) -> Option<ManualQuota> {
        self.manual_quota
    }
}

/// 本地墙钟的 Unix 秒；早于纪元或越界时为 0，聚合层会把它判成时间证据不可信。
#[must_use]
pub fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
