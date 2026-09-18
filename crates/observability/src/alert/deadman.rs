//! 死人开关：约定周期内没有收到活性回执就触发带外告警。
//!
//! 三条硬约束写在状态迁移里：
//! 1. 本地时钟回退不得静默跳过——检测到回退要报"时间证据不可信"并**仍然触发**；
//! 2. 触发有去抖与冷却，但冷却不得把连续故障压成一次都不报：
//!    冷却期内连续压制到达上限就强制升级再报一次；
//! 3. 所有时间差一律 `checked_*`，越界按"时间证据不可信"处理。

use super::{AlertMessage, AlertSeverity, AlertSummaryError, IdempotencyKey};

/// 死人开关参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeadmanPolicy {
    period_seconds: u64,
    debounce_seconds: u64,
    cooldown_seconds: u64,
    max_suppressed: u32,
}

/// 参数不合法。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("死人开关的周期、去抖、冷却或压制上限不合法")]
pub struct DeadmanPolicyError;

impl DeadmanPolicy {
    /// 周期必须 > 0；冷却可以为 0，但压制上限必须 > 0，
    /// 否则冷却就能把连续故障永远压住一次都不报。
    pub fn new(
        period_seconds: u64,
        debounce_seconds: u64,
        cooldown_seconds: u64,
        max_suppressed: u32,
    ) -> Result<Self, DeadmanPolicyError> {
        if period_seconds == 0
            || max_suppressed == 0
            || period_seconds > i64::MAX as u64
            || debounce_seconds > i64::MAX as u64
            || cooldown_seconds > i64::MAX as u64
        {
            return Err(DeadmanPolicyError);
        }
        Ok(Self {
            period_seconds,
            debounce_seconds,
            cooldown_seconds,
            max_suppressed,
        })
    }

    /// 约定回执周期。
    #[must_use]
    pub fn period_seconds(self) -> u64 {
        self.period_seconds
    }
}

/// 死人开关的持久状态。所有时刻都是 Unix 秒。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeadmanState {
    /// 最近一次活性回执时刻；`None` 表示从未收到。
    pub last_receipt: Option<i64>,
    /// 上一次观察到的本地时刻，用来发现时钟回退。
    pub last_observed: i64,
    /// 上一次真正发出告警的时刻。
    pub last_alert: Option<i64>,
    /// 自上次发出告警以来，被冷却压制掉的次数。
    pub suppressed_since_alert: u32,
    /// 故障情节序号。同一情节复用同一把幂等键，跨重启不变。
    pub episode: u64,
    /// 调度轮次。每次真正触发 +1，交给告警调度器当审计事件标识。
    pub dispatch_round: u64,
}

impl DeadmanState {
    /// 以 `now` 为锚点的初始状态：还没有回执，也还没有告警过。
    #[must_use]
    pub fn started_at(now: i64) -> Self {
        Self {
            last_receipt: None,
            last_observed: now,
            last_alert: None,
            suppressed_since_alert: 0,
            episode: 0,
            dispatch_round: 0,
        }
    }

    /// 记录一次活性回执。回执时刻晚于 `last_observed` 时同步推进观察时刻。
    #[must_use]
    pub fn with_receipt(mut self, at: i64) -> Self {
        self.last_receipt = Some(at);
        if at > self.last_observed {
            self.last_observed = at;
        }
        self.suppressed_since_alert = 0;
        self
    }
}

/// 触发原因的固定分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeadmanReason {
    /// 约定周期内没有收到活性回执。
    NoReceipt,
    /// 本地时钟相对已观察时刻回退，时间证据不可信。
    ClockRolledBack,
    /// 冷却期内连续压制已达上限，强制升级再报。
    EscalatedDuringCooldown,
}

impl DeadmanReason {
    /// 稳定机器码。
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::NoReceipt => "no-receipt",
            Self::ClockRolledBack => "clock-rolled-back",
            Self::EscalatedDuringCooldown => "escalated-during-cooldown",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::NoReceipt => "约定周期内没有收到活性回执",
            Self::ClockRolledBack => "本地时钟相对已观察时刻回退，时间证据不可信",
            Self::EscalatedDuringCooldown => "冷却期内连续故障已达压制上限，强制升级",
        }
    }
}

/// 一次评估的结论。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeadmanDecision {
    /// 回执在周期内，一切正常。
    Healthy,
    /// 已超期但还在去抖窗口内，本轮不报。
    Debouncing {
        /// 已超出约定周期多少秒。
        overdue_seconds: u64,
    },
    /// 本轮触发告警。
    Fire {
        /// 触发原因。
        reason: DeadmanReason,
        /// 待发送的告警消息。
        alert: AlertMessage,
    },
    /// 处于冷却期，本轮压制，但压制次数已计入状态。
    Suppressed {
        /// 冷却剩余秒数。
        remaining_cooldown_seconds: u64,
        /// 自上次告警以来累计压制次数。
        suppressed_count: u32,
    },
}

/// 评估失败；属于实现缺陷而非运行时状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("死人开关无法构造告警消息")]
pub struct DeadmanEvaluationError;

impl From<AlertSummaryError> for DeadmanEvaluationError {
    fn from(_: AlertSummaryError) -> Self {
        Self
    }
}

/// 死人开关的固定告警机器码。
pub const DEADMAN_ALERT_CODE: &str = "deadman-switch";

/// 评估一次；返回新的持久状态与本轮结论。
///
/// 调用方必须先把返回的新状态持久化，再发送告警：这样跨重启只会重复一次
/// 同一把幂等键的投递，由 [`super::AlertDispatcher`] 去重，而不会漏报。
pub fn evaluate(
    policy: DeadmanPolicy,
    state: DeadmanState,
    now: i64,
) -> Result<(DeadmanState, DeadmanDecision), DeadmanEvaluationError> {
    // 1. 时钟回退：不静默跳过，直接开新情节并触发。
    if now < state.last_observed {
        let mut next = state;
        next.last_observed = now;
        next.episode = state.episode.saturating_add(1);
        next.last_alert = Some(now);
        next.suppressed_since_alert = 0;
        next.dispatch_round = state.dispatch_round.saturating_add(1);
        let alert = message(DeadmanReason::ClockRolledBack, next.episode)?;
        return Ok((
            next,
            DeadmanDecision::Fire {
                reason: DeadmanReason::ClockRolledBack,
                alert,
            },
        ));
    }

    let mut next = state;
    next.last_observed = now;

    // 2. 超期判定；从未收到回执时以状态起点为锚，不当作"刚刚收到"。
    let anchor = state.last_receipt.unwrap_or(state.last_observed);
    let Some(elapsed) = now
        .checked_sub(anchor)
        .and_then(|value| u64::try_from(value).ok())
    else {
        // 差值越界：时间证据不可信，按回退同样处理，绝不静默放过。
        next.episode = state.episode.saturating_add(1);
        next.last_alert = Some(now);
        next.suppressed_since_alert = 0;
        next.dispatch_round = state.dispatch_round.saturating_add(1);
        let alert = message(DeadmanReason::ClockRolledBack, next.episode)?;
        return Ok((
            next,
            DeadmanDecision::Fire {
                reason: DeadmanReason::ClockRolledBack,
                alert,
            },
        ));
    };
    if elapsed <= policy.period_seconds {
        next.suppressed_since_alert = 0;
        return Ok((next, DeadmanDecision::Healthy));
    }
    let overdue = elapsed - policy.period_seconds;
    if overdue < policy.debounce_seconds {
        return Ok((
            next,
            DeadmanDecision::Debouncing {
                overdue_seconds: overdue,
            },
        ));
    }

    // 3. 冷却与强制升级。
    if let Some(last_alert) = state.last_alert {
        let since = now
            .checked_sub(last_alert)
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(u64::MAX);
        if since < policy.cooldown_seconds {
            if state.suppressed_since_alert >= policy.max_suppressed {
                next.episode = state.episode.saturating_add(1);
                next.last_alert = Some(now);
                next.suppressed_since_alert = 0;
                next.dispatch_round = state.dispatch_round.saturating_add(1);
                let alert = message(DeadmanReason::EscalatedDuringCooldown, next.episode)?;
                return Ok((
                    next,
                    DeadmanDecision::Fire {
                        reason: DeadmanReason::EscalatedDuringCooldown,
                        alert,
                    },
                ));
            }
            next.suppressed_since_alert = state.suppressed_since_alert.saturating_add(1);
            return Ok((
                next,
                DeadmanDecision::Suppressed {
                    remaining_cooldown_seconds: policy.cooldown_seconds - since,
                    suppressed_count: next.suppressed_since_alert,
                },
            ));
        }
    }

    if state.last_alert.is_none() {
        next.episode = state.episode.saturating_add(1);
    }
    next.last_alert = Some(now);
    next.suppressed_since_alert = 0;
    next.dispatch_round = state.dispatch_round.saturating_add(1);
    let alert = message(DeadmanReason::NoReceipt, next.episode)?;
    Ok((
        next,
        DeadmanDecision::Fire {
            reason: DeadmanReason::NoReceipt,
            alert,
        },
    ))
}

fn message(reason: DeadmanReason, episode: u64) -> Result<AlertMessage, AlertSummaryError> {
    let episode_text = episode.to_string();
    let key = IdempotencyKey::derive("deadman-switch", &[reason.code(), episode_text.as_str()]);
    AlertMessage::new(
        key,
        AlertSeverity::Critical,
        DEADMAN_ALERT_CODE,
        format!("{}（情节 {episode}）", reason.summary()),
    )
}
