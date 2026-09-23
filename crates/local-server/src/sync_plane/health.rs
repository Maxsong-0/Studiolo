//! 平面的健康证据出口：第六节 `BridgeEvidence`、owner 活性、Sync 配额读数。
//!
//! 本文件只累积事实，不读磁盘；`startup/health_input.rs` 经 `BridgeHealthProbe` 取快照。

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use studiolo_host_config::sync_transport::ManualQuota;
use studiolo_observability::health::{BridgeEvidence, OwnerLabel, OwnerLiveness};
use studiolo_observability::quota::SyncUsage;

use super::driver::RoundOutcome;

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

    /// 记录一轮结束；`outcome` 决定退出分类、恢复状态与守卫状态。
    pub fn record_round(&self, now_unix: i64, outcome: &Result<RoundOutcome, super::PlaneError>) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        inner.last_round_unix = Some(now_unix);
        // B 轮 W5 实现：按 outcome 更新 evidence.{last_batch, fused, unknown_jobs,
        // quiescence_valid, conflict_copies, mirror_bytes, last_exit, recovery, residue_guard, mode}
        // 与 usage；这里先只记时刻。
        let _ = outcome;
    }

    /// 驱动器在观察镜像后回填的字节数与对象数。
    pub fn record_usage(&self, usage: SyncUsage) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.usage = Some(usage);
            inner.evidence.mirror_bytes = Some(usage.bytes);
        }
    }
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
            last_receipt: last.map(studiolo_observability::health::Instant::from_unix_seconds),
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
