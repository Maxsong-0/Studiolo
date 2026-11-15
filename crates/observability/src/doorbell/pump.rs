//! 门铃的一次运行：从上次的位置往后读账本，把白名单内的事件按顺序发出去。
//!
//! 顺序是硬约束：先写 Intent 审计，写不进去就**不发**；发完再写 Outcome；送到了才往前挪位置。
//! 前一条没送到，后面的不发，整条通道一起退避。本模块不睡眠、不起线程、不读时钟，
//! 「现在几点」由调用方传入，重试靠定时器下一次拉起。
//!
//! 「送到没有」以账本里的结果记录为准，状态文件只是位置和退避的缓存：状态没存上就崩溃、
//! 甚至状态文件丢了，重扫时送到过的不重发，没送到的不会被跳过。每条事件全程持账本的操作许可。

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
/// 启用标记的固定事件编号。
const ENABLED_EVENT_ID: &str = "doorbell-enabled";
/// 过期记录占用的轮次名（不是数字，和发送轮次不会撞）。
const EXPIRED_ROUND: &str = "expired";

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
        // 启用本身进审计，这条记录的位置就是起点，之前的历史不补发。事件编号固定：
        // 记了启用却没存下状态（崩溃，或状态文件后来丢了）时，重试拿回的是原来那条记录，
        // 位置回到原起点重新扫一遍。送到过、过期过的账本里都有结论，不会重发；
        // 两次之间新来的事件也不会被一个更晚的起点跳过去。
        let _operation = ledger.enter_operation()?;
        let enabled = ledger.append(enabled_event()?)?;
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
            // 一条事件从查账、记意图、发送、记结果到存位置是一次完整操作，全程持操作许可
            // （`AuditLedger::enter_operation` 的调用方义务）：备份封存要么等这一条做完，
            // 要么让它整条不开始。许可一条一拿，不跨事件，封存最多等一次发送的超时。
            let _operation = ledger.enter_operation()?;
            match plan(ledger, &event, state.attempts, now_ms)? {
                Plan::Settled => {}
                Plan::Expire => {
                    ledger.append(dispatch_event(
                        &event,
                        EXPIRED_ROUND,
                        AuditPhase::Outcome,
                        "expired",
                    )?)?;
                    summary.expired += 1;
                }
                Plan::Send(_) if summary.delivered == MAX_DELIVERIES_PER_RUN => break 'pages,
                Plan::Send(round) => {
                    let round = round.to_string();
                    ledger.append(dispatch_event(
                        &event,
                        &round,
                        AuditPhase::Intent,
                        "intent",
                    )?)?;
                    let result = transport.post(
                        event.message_id(),
                        now_ms.div_euclid(1000),
                        &event.body(transport.link()),
                    );
                    if result == Err(PostError::Gone) {
                        // 先存「停用」再记结果。反过来的话，记了结果、没存状态就崩溃，
                        // 重跑会换一个轮次，再去敲一次已经说不要了的门。
                        state.gone = true;
                        store.save(&state)?;
                    }
                    let classification =
                        result.map_or_else(PostError::classification, |()| "delivered");
                    ledger.append(dispatch_event(
                        &event,
                        &round,
                        AuditPhase::Outcome,
                        classification,
                    )?)?;
                    match result {
                        Ok(()) => summary.delivered += 1,
                        Err(PostError::Gone) => {
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

/// 对一条白名单事件接下来该做什么。
enum Plan {
    /// 账本里已有定论（送到过，或记过过期）：只把位置挪过去，不发也不再记。
    Settled,
    /// 超过年龄上限还没送出去：记一条过期，跳过。
    Expire,
    /// 用这个轮次编号发一次。
    Send(u32),
}

/// 账本是「送到没有」的权威，状态文件只是位置：上一次运行可能已经发了、只是没来得及存状态
/// 就退出了，状态文件丢了重扫时也一样。送到过的不再发；没送到的换下一个轮次编号，
/// 不去改写已有的审计。
fn plan(
    ledger: &dyn AuditLedger,
    event: &DoorbellEvent,
    attempts: u32,
    now_ms: i64,
) -> Result<Plan, AuditError> {
    let mut round = attempts;
    while let Some(classification) = recorded(ledger, event, &round.to_string())? {
        if classification == "delivered" {
            return Ok(Plan::Settled);
        }
        round = round.checked_add(1).ok_or(AuditError::Unavailable)?;
        if round - attempts > MAX_ATTEMPT_PROBES {
            return Err(AuditError::Unavailable);
        }
    }
    if recorded(ledger, event, EXPIRED_ROUND)?.is_some() {
        return Ok(Plan::Settled);
    }
    if now_ms.saturating_sub(event.occurred_at_ms()) > MAX_EVENT_AGE_MS {
        return Ok(Plan::Expire);
    }
    Ok(Plan::Send(round))
}

/// 这条事件在某个轮次上已经记下的结论；没有结果记录返回 `None`。
fn recorded(
    ledger: &dyn AuditLedger,
    event: &DoorbellEvent,
    round: &str,
) -> Result<Option<String>, AuditError> {
    let outcome = ledger.query(
        &AuditFilter::default()
            .with_event_id(dispatch_id(event, round)?)
            .with_phase(AuditPhase::Outcome),
    )?;
    Ok(outcome.first().map(|record| {
        record.event().payload().as_value()["classification"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }))
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

/// 启用标记：编号和载荷都固定，重复追加拿回的是第一次那条记录（启用时间看记录自己的时间）。
fn enabled_event() -> Result<AuditEvent, AuditError> {
    Ok(AuditEvent::new(
        AuditEventId::parse(ENABLED_EVENT_ID)?,
        AuditPhase::Outcome,
        DOORBELL_ENABLED_ACTION,
        SanitizedPayload::try_from(serde_json::json!({ "schema": 1 }))?,
    ))
}

#[cfg(test)]
#[path = "../../tests/support/doorbell_pump.rs"]
mod tests;
