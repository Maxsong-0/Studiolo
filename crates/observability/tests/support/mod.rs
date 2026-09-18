#![allow(dead_code)]

//! 契约测试共用的本地假件。
//!
//! 这里**没有任何真实收件地址或凭据**，也不连接任何外部服务。

use std::sync::Mutex;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord, MemoryAuditLedger,
};
use studiolo_observability::alert::{
    AlertMessage, AlertTransport, DeliveryReceipt, TransportError,
};

/// 本地假 transport：按脚本逐次返回结果，记录被调用次数。
pub struct ScriptedTransport {
    script: Mutex<Vec<Result<(), TransportError>>>,
    calls: Mutex<usize>,
}

impl ScriptedTransport {
    /// 脚本按顺序消费；用尽后重复最后一项。
    pub fn new(script: Vec<Result<(), TransportError>>) -> Self {
        Self {
            script: Mutex::new(script),
            calls: Mutex::new(0),
        }
    }

    /// 永远成功。
    pub fn always_ok() -> Self {
        Self::new(vec![Ok(())])
    }

    /// 永远返回同一个错误。
    pub fn always(error: TransportError) -> Self {
        Self::new(vec![Err(error)])
    }

    /// 实际被调用的次数。
    pub fn calls(&self) -> usize {
        self.calls.lock().map_or(usize::MAX, |value| *value)
    }
}

impl AlertTransport for ScriptedTransport {
    fn deliver(&self, message: &AlertMessage) -> Result<DeliveryReceipt, TransportError> {
        let mut calls = match self.calls.lock() {
            Ok(value) => value,
            Err(_) => return Err(TransportError::Unavailable),
        };
        *calls += 1;
        let index = *calls - 1;
        let script = match self.script.lock() {
            Ok(value) => value,
            Err(_) => return Err(TransportError::Unavailable),
        };
        let step = script
            .get(index)
            .or_else(|| script.last())
            .copied()
            .unwrap_or(Err(TransportError::Unavailable));
        step.map(|()| DeliveryReceipt::new(message.key().clone()))
    }
}

/// 永远不可用的审计；用来证明 Intent 写不进去就一次都不发。
pub struct BrokenLedger;

impl AuditLedger for BrokenLedger {
    fn append(&self, _event: AuditEvent) -> Result<AuditRecord, AuditError> {
        Err(AuditError::Unavailable)
    }

    fn query(&self, _filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        Err(AuditError::Unavailable)
    }
}

/// 可用的内存审计。
pub fn working_ledger() -> MemoryAuditLedger {
    MemoryAuditLedger::new()
}
