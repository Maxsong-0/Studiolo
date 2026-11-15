//! 门铃的一次运行：从上次的位置往后读账本，把白名单内的事件按顺序发出去。
//!
//! 顺序是硬约束：先写 Intent 审计，写不进去就**不发**；发完再写 Outcome；送到了才往前挪位置。
//! 前一条没送到，后面的不发，整条通道一起退避。本模块不睡眠、不起线程、不读时钟，
//! 「现在几点」由调用方传入，重试靠定时器下一次拉起。

use super::{DoorbellEvent, DoorbellTransport, PostError, classify};
use crate::store::{DoorbellStore, StoreError};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, SanitizedPayload,
};

/// 每次投递的固定审计动作名。
pub const DOORBELL_AUDIT_ACTION: &str = "observability.doorbell.dispatch";
/// 启用门铃时记的审计动作名；位置从这一条开始，不补发之前的历史。
pub const DOORBELL_ENABLED_ACTION: &str = "observability.doorbell.enabled";
/// 超过这个年龄还没送出去的事件直接跳过：门铃晚一天响没有意义。
pub const MAX_EVENT_AGE_MS: i64 = 24 * 60 * 60 * 1000;

/// 一次查询读多少条账本记录。
const PAGE: usize = 256;
/// 一次运行最多看多少条记录、最多发多少条事件；剩下的留给下一次拉起。
const MAX_RECORDS_PER_RUN: usize = 16 * PAGE;
const MAX_DELIVERIES_PER_RUN: u32 = 32;
/// 第 n 次失败后等多久再试（分钟）；超过表长按最后一项。
const BACKOFF_MINUTES: [i64; 6] = [0, 5, 30, 120, 300, 600];
/// 同一条事件在账本里最多找多少个已用过的轮次编号。
const MAX_ATTEMPT_PROBES: u32 = 64;

/// 门铃的持久状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoorbellState {
    /// 已处理（送到、不在白名单或已过期）的最后一条账本记录的位置。
    pub cursor: u64,
    /// 队首事件已经失败的次数；送到或跳过后归零。
    pub attempts: u32,
    /// 这个时刻（毫秒）之前不再尝试。
    pub retry_at_ms: i64,
    /// 收件方回过 410；换收件地址之前不再发。
    pub gone: bool,
    /// 上次使用的收件地址指纹，用来发现「换了地址」。
    pub endpoint_tag: String,
}

/// 状态存储接缝。真实实现是 [`DoorbellStore`]。
pub trait StateStore {
    /// 读取状态；没启用过返回 `Ok(None)`，损坏返回错误。
    fn load(&self) -> Result<Option<DoorbellState>, StoreError>;
    /// 完整替换状态。
    fn save(&self, state: &DoorbellState) -> Result<(), StoreError>;
}

impl StateStore for DoorbellStore {
    fn load(&self) -> Result<Option<DoorbellState>, StoreError> {
        DoorbellStore::load(self)
    }

    fn save(&self, state: &DoorbellState) -> Result<(), StoreError> {
        DoorbellStore::save(self, state)
    }
}

/// 一次运行停在哪。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// 第一次运行：记下启用位置，什么都没发。
    Enabled,
    /// 读到了账本末尾（或本次上限），没有失败。
    CaughtUp,
    /// 还在退避期内，本次没尝试。
    Backoff,
    /// 队首事件这次没送到，附固定失败分类；已排好下次重试时间。
    Deferred(&'static str),
    /// 收件方回了 410，已停用到换地址为止。
    EndpointGone,
}

/// 一次运行的结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunSummary {
    /// 停在哪。
    pub outcome: RunOutcome,
    /// 本次送到几条。
    pub delivered: u32,
    /// 本次因过期跳过几条。
    pub expired: u32,
}

/// 一次运行没法进行的原因；都没有发出任何没记审计的请求。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    /// 收件端没配。
    #[error("门铃收件端未配置")]
    NotConfigured,
    /// 收件配置不合法。
    #[error("门铃收件配置不合法")]
    Malformed,
    /// 状态目录不可用或状态损坏；不当成「从头开始」。
    #[error("门铃状态不可用")]
    StateUnavailable,
    /// 审计账本不可用；Intent 写不进去就一次都没发。
    #[error("审计不可用")]
    AuditUnavailable,
}

impl From<StoreError> for RunError {
    fn from(_: StoreError) -> Self {
        Self::StateUnavailable
    }
}

impl From<AuditError> for RunError {
    fn from(_: AuditError) -> Self {
        Self::AuditUnavailable
    }
}

/// 跑一次。`now_ms` 是当前时间的毫秒时间戳。
pub fn run(
    ledger: &dyn AuditLedger,
    store: &dyn StateStore,
    transport: &dyn DoorbellTransport,
    now_ms: i64,
) -> Result<RunSummary, RunError> {
    let tag = transport.endpoint_tag().map_err(|error| match error {
        PostError::NotConfigured => RunError::NotConfigured,
        _ => RunError::Malformed,
    })?;
    let mut summary = RunSummary {
        outcome: RunOutcome::CaughtUp,
        delivered: 0,
        expired: 0,
    };
    let Some(mut state) = store.load()? else {
        // 启用本身进审计；这条记录的位置就是起点，之前的历史不补发。
        let enabled = ledger.append(enabled_event(now_ms)?)?;
        store.save(&DoorbellState {
            cursor: enabled.offset(),
            attempts: 0,
            retry_at_ms: 0,
            gone: false,
            endpoint_tag: tag,
        })?;
        summary.outcome = RunOutcome::Enabled;
        return Ok(summary);
    };
    if state.endpoint_tag != tag {
        // 换了收件地址：旧地址的停用和退避不带过去，位置不动。
        state = DoorbellState {
            cursor: state.cursor,
            attempts: 0,
            retry_at_ms: 0,
            gone: false,
            endpoint_tag: tag,
        };
        store.save(&state)?;
    }
    if state.gone {
        summary.outcome = RunOutcome::EndpointGone;
        return Ok(summary);
    }
    if now_ms < state.retry_at_ms {
        summary.outcome = RunOutcome::Backoff;
        return Ok(summary);
    }
    let mut saved = state.cursor;
    let mut scanned = 0;
    'pages: loop {
        let filter = AuditFilter::default()
            .after_offset(state.cursor)
            .with_limit(PAGE);
        let records = ledger.query(&filter)?;
        for record in &records {
            scanned += 1;
            let Some(event) = classify(record) else {
                state.cursor = record.offset();
                continue;
            };
            if state.cursor != saved {
                // 先把跳过的进度落盘：发送中途崩了，下次不必重扫，位置也不会退回去。
                store.save(&state)?;
                saved = state.cursor;
            }
            if now_ms.saturating_sub(event.occurred_at_ms()) > MAX_EVENT_AGE_MS {
                ledger.append(dispatch_event(
                    &event,
                    "expired",
                    AuditPhase::Outcome,
                    "expired",
                )?)?;
                summary.expired += 1;
            } else if summary.delivered == MAX_DELIVERIES_PER_RUN {
                break 'pages;
            } else {
                match deliver(ledger, transport, &event, state.attempts, now_ms)? {
                    Ok(()) => summary.delivered += 1,
                    Err(PostError::Gone) => {
                        state.gone = true;
                        store.save(&state)?;
                        summary.outcome = RunOutcome::EndpointGone;
                        return Ok(summary);
                    }
                    Err(error) => {
                        state.attempts = state.attempts.saturating_add(1);
                        state.retry_at_ms = now_ms.saturating_add(backoff_ms(state.attempts));
                        store.save(&state)?;
                        summary.outcome = RunOutcome::Deferred(error.classification());
                        return Ok(summary);
                    }
                }
            }
            state.cursor = record.offset();
            state.attempts = 0;
            state.retry_at_ms = 0;
            store.save(&state)?;
            saved = state.cursor;
        }
        if records.len() < PAGE || scanned >= MAX_RECORDS_PER_RUN {
            break;
        }
    }
    if state.cursor != saved {
        store.save(&state)?;
    }
    Ok(summary)
}

/// 发一条：审计先行；外层错误是审计不可用（一次都没发，或结果没记上），内层是投递结论。
fn deliver(
    ledger: &dyn AuditLedger,
    transport: &dyn DoorbellTransport,
    event: &DoorbellEvent,
    attempts: u32,
    now_ms: i64,
) -> Result<Result<(), PostError>, AuditError> {
    // 上一次运行可能已经发了、只是没来得及记状态就退出了：账本里这一轮已有结果的，
    // 送到了就不再发；没送到的换下一个轮次编号，不去改写已有的审计。
    let mut round = attempts;
    loop {
        let recorded = ledger.query(
            &AuditFilter::default()
                .with_event_id(dispatch_id(event, &round.to_string())?)
                .with_phase(AuditPhase::Outcome),
        )?;
        let Some(outcome) = recorded.first() else {
            break;
        };
        if outcome.event().payload().as_value()["classification"] == "delivered" {
            return Ok(Ok(()));
        }
        round = round.checked_add(1).ok_or(AuditError::Unavailable)?;
        if round - attempts > MAX_ATTEMPT_PROBES {
            return Err(AuditError::Unavailable);
        }
    }
    let round = round.to_string();
    ledger.append(dispatch_event(event, &round, AuditPhase::Intent, "intent")?)?;
    let result = transport.post(
        event.message_id(),
        now_ms.div_euclid(1000),
        &event.body(transport.link()),
    );
    let classification = result.map_or_else(PostError::classification, |()| "delivered");
    ledger.append(dispatch_event(
        event,
        &round,
        AuditPhase::Outcome,
        classification,
    )?)?;
    Ok(result)
}

fn backoff_ms(failures: u32) -> i64 {
    let index = usize::try_from(failures.saturating_sub(1))
        .unwrap_or(usize::MAX)
        .min(BACKOFF_MINUTES.len() - 1);
    BACKOFF_MINUTES[index] * 60 * 1000
}

fn dispatch_id(event: &DoorbellEvent, round: &str) -> Result<AuditEventId, AuditError> {
    AuditEventId::parse(format!("doorbell-{}-{round}", event.message_id()))
}

/// 审计载荷只放固定机器码与派生编号；不放收件地址、正文或密钥。
fn dispatch_event(
    event: &DoorbellEvent,
    round: &str,
    phase: AuditPhase,
    classification: &str,
) -> Result<AuditEvent, AuditError> {
    let payload = SanitizedPayload::try_from(serde_json::json!({
        "event_type": event.kind().code(),
        "message_id": event.message_id(),
        "source_offset": event.source_offset(),
        "round": round,
        "classification": classification,
    }))?;
    Ok(AuditEvent::new(
        dispatch_id(event, round)?,
        phase,
        DOORBELL_AUDIT_ACTION,
        payload,
    ))
}

fn enabled_event(now_ms: i64) -> Result<AuditEvent, AuditError> {
    Ok(AuditEvent::new(
        AuditEventId::parse(format!("doorbell-enabled-{now_ms}"))?,
        AuditPhase::Outcome,
        DOORBELL_ENABLED_ACTION,
        SanitizedPayload::try_from(serde_json::json!({ "enabled_at_ms": now_ms }))?,
    ))
}

#[cfg(test)]
#[path = "../../tests/support/doorbell_pump.rs"]
mod tests;
