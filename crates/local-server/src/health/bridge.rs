//! 仅搬运已装配探针的封闭字段；镜像读数与人工账号记录分别呈现。
use serde::Serialize;
use studiolo_host_config::sync_transport::ManualQuota;
use studiolo_observability::health::{
    BridgeEvidence, ExitClass, HealthSection, HealthState, RecoveryState, ResidueGuard,
};

/// 人工账号记录有效期；过期仍展示旧记录，但健康结论必须未知。
pub const MANUAL_QUOTA_MAX_AGE_SECONDS: i64 = 86_400;

/// 最近闭合批次的序号和时刻。
#[derive(Debug, Serialize)]
pub struct BatchBody {
    pub(super) sequence: String,
    pub(super) closed_at: Option<String>,
}

/// 一轮要比对的路径数与上限（BUG-047）；两个数都按十进制字符串传输。
#[derive(Debug, Serialize)]
pub struct PathUsageBody {
    pub(super) used: String,
    pub(super) budget: String,
}

/// 不含路径、凭证、远端地址或stderr的Bridge投影；u64按十进制字符串传输。
#[derive(Debug, Serialize)]
pub struct BridgeBody {
    pub(super) last_batch: Option<BatchBody>,
    pub(super) fused: Option<bool>,
    pub(super) unknown_jobs: Option<String>,
    pub(super) quiescence_valid: Option<bool>,
    pub(super) conflict_copies: Option<String>,
    pub(super) mirror_bytes: Option<String>,
    pub(super) last_exit: Option<&'static str>,
    pub(super) recovery: Option<&'static str>,
    pub(super) residue_guard: Option<&'static str>,
    pub(super) mode: Option<&'static str>,
    pub(super) config_matches: Option<bool>,
    pub(super) deletes_blocked: Option<String>,
    pub(super) path_usage: Option<PathUsageBody>,
}
impl BridgeBody {
    pub(super) fn from_evidence(value: BridgeEvidence) -> Self {
        Self {
            last_batch: value.last_batch.map(|stamp| BatchBody {
                sequence: stamp.sequence.to_string(),
                closed_at: timestamp(stamp.unix_seconds),
            }),
            fused: value.fused,
            unknown_jobs: value.unknown_jobs.map(|n| n.to_string()),
            quiescence_valid: value.quiescence_valid,
            conflict_copies: value.conflict_copies.map(|n| n.to_string()),
            mirror_bytes: value.mirror_bytes.map(|n| n.to_string()),
            last_exit: value.last_exit.map(|value| match value {
                ExitClass::Clean => "clean",
                ExitClass::NonZero => "non_zero",
                ExitClass::Signal => "signal",
                ExitClass::DeadlineExceeded => "deadline_exceeded",
                ExitClass::OwnerStopped => "owner_stopped",
                ExitClass::Unconfirmed => "unconfirmed",
            }),
            recovery: value.recovery.map(|value| match value {
                RecoveryState::Idle => "idle",
                RecoveryState::ForcedMirrorRemote => "forced_mirror_remote",
                RecoveryState::Backoff => "backoff",
            }),
            residue_guard: value.residue_guard.map(|value| match value {
                ResidueGuard::Clear => "clear",
                ResidueGuard::Blocked => "blocked",
            }),
            mode: value.mode.map(|value| value.label()),
            config_matches: value.config_matches,
            deletes_blocked: value.deletes_blocked.map(|n| n.to_string()),
            path_usage: value.path_usage.map(|usage| PathUsageBody {
                used: usage.used.to_string(),
                budget: usage.budget.to_string(),
            }),
        }
    }
}

/// 人工抄录值不是实时账号用量；时间无效时保留数值但明确显示未知。
#[derive(Debug, Serialize)]
pub struct ManualQuotaBody {
    pub(super) account_limit_bytes: String,
    pub(super) used_bytes_manual: String,
    pub(super) recorded_at: Option<String>,
    pub(super) as_of: Option<String>,
    pub(super) warning_percent: u8,
    pub(super) max_age_seconds: i64,
    pub(super) state: &'static str,
    pub(super) reason: &'static str,
}
impl ManualQuotaBody {
    pub(super) fn new(value: ManualQuota, now: i64) -> Self {
        let (state, reason, _) = verdict(Some(value), now);
        Self {
            account_limit_bytes: value.account_limit_bytes.to_string(),
            used_bytes_manual: value.used_bytes_manual.to_string(),
            recorded_at: timestamp(value.recorded_at_unix),
            as_of: timestamp(now),
            warning_percent: 70,
            max_age_seconds: MANUAL_QUOTA_MAX_AGE_SECONDS,
            state: state.code(),
            reason,
        }
    }
}

pub(super) fn quota_section(quota: Option<ManualQuota>, now: i64) -> HealthSection {
    let (state, _, detail) = verdict(quota, now);
    // 文案封闭且短，不含外部输入；构造期仍检查既有脱敏契约。
    HealthSection::new(
        "manual-sync-quota",
        "账号配额（人工记录）",
        state,
        detail.to_owned(),
    )
    .expect("固定健康文案必须满足脱敏契约")
}

fn verdict(quota: Option<ManualQuota>, now: i64) -> (HealthState, &'static str, &'static str) {
    let Some(quota) = quota else {
        return (
            HealthState::Unknown,
            "missing",
            "尚无人工账号记录，实时账号用量未知；镜像大小不能替代账号用量",
        );
    };
    if quota.account_limit_bytes == 0 {
        return (
            HealthState::Unknown,
            "invalid",
            "人工账号上限无效，请重新核对记录",
        );
    }
    let age = now.checked_sub(quota.recorded_at_unix);
    if now <= 0
        || quota.recorded_at_unix <= 0
        || age.is_none_or(|n| n < 0)
        || timestamp(now).is_none()
        || timestamp(quota.recorded_at_unix).is_none()
    {
        return (
            HealthState::Unknown,
            "clock",
            "人工记录的时刻无法核验，请检查时钟并更新记录",
        );
    }
    if age.is_some_and(|n| n > MANUAL_QUOTA_MAX_AGE_SECONDS) {
        return (
            HealthState::Unknown,
            "stale",
            "人工账号记录已超过24小时，请核对并更新；当前用量未知",
        );
    }
    if quota.used_bytes_manual > quota.account_limit_bytes {
        (
            HealthState::Degraded,
            "exceeded",
            "人工记录已超过账号上限；请在Obsidian设置中核对，实时用量未知",
        )
    } else if u128::from(quota.used_bytes_manual) * 100
        >= u128::from(quota.account_limit_bytes) * 70
    {
        (
            HealthState::Degraded,
            "warning",
            "人工记录已达到70%预警；请在Obsidian设置中核对，实时用量未知",
        )
    } else {
        (
            HealthState::Normal,
            "within",
            "人工记录低于70%预警；这是抄录时的值，实时账号用量未知",
        )
    }
}
fn timestamp(value: i64) -> Option<String> {
    if value <= 0 {
        return None;
    }
    jiff::Timestamp::from_second(value)
        .ok()
        .map(|value| value.to_string())
}
