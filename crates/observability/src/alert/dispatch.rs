//! 告警调度：审计先于发送 ＋ 幂等键去重 ＋ 有界重试 ＋ 固定失败分类。
//!
//! 顺序是硬约束：先写 Intent 审计，写不进去就**不发**（失败关闭）；
//! 发完再写 Outcome。调度器不睡眠、不起线程，重试次数有硬上限，
//! 便于把"超时""不可用""审计不可用"这些失败契约写成确定性测试。
//!
//! payload 只放固定机器码、等级、幂等键与失败分类，
//! 不放告警正文、收件地址、凭据或任何 token 片段。

use super::{AlertMessage, AlertTransport, IdempotencyKey, TransportError};
use std::collections::{BTreeSet, VecDeque};
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditLedger, AuditPhase, SanitizedPayload};

/// 带外告警的固定审计动作名。
pub const ALERT_AUDIT_ACTION: &str = "observability.alert.dispatch";

/// 一次调度的结论。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// 本次真实送达，附实际尝试次数。
    Delivered {
        /// 直到成功为止的尝试次数，至少为 1。
        attempts: u32,
    },
    /// 同一把幂等键此前已送达，本次不再重复发送。
    AlreadyDelivered,
    /// 发送端未配置。这是明确的"未配置"，不是成功。
    NotConfigured,
    /// 审计不可用；Intent 没写进去就**一次都没发**。
    AuditUnavailable,
    /// 有界重试用尽或遇到不可重试错误。
    Failed {
        /// 固定失败分类，可核对，且不含任何 token 片段。
        classification: &'static str,
        /// 实际尝试次数。
        attempts: u32,
    },
}

/// 告警调度器。记住已送达的幂等键，容量有界，满了按先进先出淘汰。
#[derive(Debug)]
pub struct AlertDispatcher {
    max_attempts: u32,
    capacity: usize,
    delivered: BTreeSet<IdempotencyKey>,
    order: VecDeque<IdempotencyKey>,
}

/// 调度器参数不合法。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("告警调度器的重试次数或去重容量不合法")]
pub struct DispatcherConfigError;

impl AlertDispatcher {
    /// `max_attempts` 与 `capacity` 都必须在 1..=1024；没有"无限重试"。
    pub fn new(max_attempts: u32, capacity: usize) -> Result<Self, DispatcherConfigError> {
        if !(1..=1024).contains(&max_attempts) || !(1..=1024).contains(&capacity) {
            return Err(DispatcherConfigError);
        }
        Ok(Self {
            max_attempts,
            capacity,
            delivered: BTreeSet::new(),
            order: VecDeque::new(),
        })
    }

    /// 已记住的送达键数量。
    #[must_use]
    pub fn remembered(&self) -> usize {
        self.delivered.len()
    }

    /// 是否已记住这把键。
    #[must_use]
    pub fn contains(&self, key: &IdempotencyKey) -> bool {
        self.delivered.contains(key)
    }

    /// 从持久状态或既有审计恢复已送达的键；跨重启不重复打扰。
    pub fn remember(&mut self, key: IdempotencyKey) {
        if !self.delivered.insert(key.clone()) {
            return;
        }
        self.order.push_back(key);
        while self.order.len() > self.capacity {
            if let Some(evicted) = self.order.pop_front() {
                self.delivered.remove(&evicted);
            }
        }
    }

    /// 调度一条告警。可重试错误按 `max_attempts` 有界重试，其余立即失败。
    ///
    /// Intent 审计写失败时直接返回 [`DispatchOutcome::AuditUnavailable`]，
    /// transport 一次都不会被调用。Outcome 审计写失败不改变已发生的事实，
    /// 但会把结论降级成 [`DispatchOutcome::AuditUnavailable`]，
    /// 由调用方按"结果未知"处理，而不是当作确定成功。
    /// `round` 是调用方**持久**的调度轮次：同一轮内的 transport 重试共享同一条
    /// 审计事件，不同轮各自留痕，因此重复调度不会撞上幂等冲突。
    pub fn dispatch(
        &mut self,
        ledger: &dyn AuditLedger,
        transport: &dyn AlertTransport,
        message: &AlertMessage,
        round: u64,
    ) -> DispatchOutcome {
        if self.delivered.contains(message.key()) {
            return DispatchOutcome::AlreadyDelivered;
        }
        if record(ledger, message, round, AuditPhase::Intent, "intent", 0).is_err() {
            return DispatchOutcome::AuditUnavailable;
        }
        let outcome = self.deliver(transport, message);
        let (classification, attempts) = match &outcome {
            DispatchOutcome::Delivered { attempts } => ("delivered", *attempts),
            DispatchOutcome::AlreadyDelivered => ("already-delivered", 0),
            DispatchOutcome::NotConfigured => ("not-configured", 1),
            DispatchOutcome::AuditUnavailable => ("audit-unavailable", 0),
            DispatchOutcome::Failed {
                classification,
                attempts,
            } => (*classification, *attempts),
        };
        if record(
            ledger,
            message,
            round,
            AuditPhase::Outcome,
            classification,
            attempts,
        )
        .is_err()
        {
            return DispatchOutcome::AuditUnavailable;
        }
        outcome
    }

    fn deliver(
        &mut self,
        transport: &dyn AlertTransport,
        message: &AlertMessage,
    ) -> DispatchOutcome {
        let mut attempts = 0;
        let mut last = TransportError::Unavailable;
        while attempts < self.max_attempts {
            attempts += 1;
            match transport.deliver(message) {
                Ok(receipt) => {
                    self.remember(receipt.key().clone());
                    return DispatchOutcome::Delivered { attempts };
                }
                Err(TransportError::NotConfigured) => return DispatchOutcome::NotConfigured,
                Err(error) if !error.retryable() => {
                    return DispatchOutcome::Failed {
                        classification: error.classification(),
                        attempts,
                    };
                }
                Err(error) => last = error,
            }
        }
        DispatchOutcome::Failed {
            classification: last.classification(),
            attempts,
        }
    }
}

fn record(
    ledger: &dyn AuditLedger,
    message: &AlertMessage,
    round: u64,
    phase: AuditPhase,
    classification: &str,
    attempts: u32,
) -> Result<(), studiolo_audit_ledger::AuditError> {
    let event_id = AuditEventId::parse(format!("alert-{}-{round}", message.key().as_str()))?;
    let payload = SanitizedPayload::try_from(serde_json::json!({
        "alert_code": message.code(),
        "severity": message.severity().code(),
        "idempotency_key": message.key().as_str(),
        "classification": classification,
        "attempts": attempts,
    }))?;
    ledger.append(AuditEvent::new(
        event_id,
        phase,
        ALERT_AUDIT_ACTION,
        payload,
    ))?;
    Ok(())
}
