//! 与持久 Adapter 共用语义及编码的内存契约替身。

use crate::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
    IntegrityHash, codec,
};
use jiff::Timestamp;
use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard},
};

/// 保留精确重放、追加顺序、过滤与完整性链的内存 Adapter。
#[derive(Debug, Default)]
pub struct MemoryAuditLedger {
    state: Mutex<MemoryState>,
}

#[derive(Debug, Default)]
struct MemoryState {
    records: Vec<AuditRecord>,
    keys: BTreeMap<(AuditEventId, AuditPhase), usize>,
}

impl MemoryAuditLedger {
    /// 创建空内存账本；不执行任何文件操作。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, MemoryState>, AuditError> {
        self.state.lock().map_err(|_| AuditError::Unavailable)
    }
}

impl AuditLedger for MemoryAuditLedger {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let mut state = self.lock()?;
        let key = (event.event_id().clone(), event.phase());
        if let Some(index) = state.keys.get(&key).copied() {
            let existing = &state.records[index];
            return if existing.event() == &event {
                Ok(existing.clone())
            } else {
                Err(AuditError::IdempotencyConflict {
                    event_id: key.0,
                    phase: key.1,
                })
            };
        }
        let offset = state.records.len() as u64 + 1;
        let previous = state
            .records
            .last()
            .map_or_else(IntegrityHash::genesis, AuditRecord::record_hash);
        let record = codec::build(offset, Timestamp::now(), previous, event)?;
        let index = state.records.len();
        state.keys.insert(key, index);
        state.records.push(record.clone());
        Ok(record)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        Ok(self
            .lock()?
            .records
            .iter()
            .filter(|record| filter.matches(record))
            .take(filter.limit().unwrap_or(usize::MAX))
            .cloned()
            .collect())
    }
}
