//! 只读派生待投事件；确认消费精确候选，直到 Outcome ACK 才整理生命周期标记。

use super::{ContentAuditCandidate, ContentAuditPhase, store as events};
use crate::{BrokerError, FsVaultBroker, history::store, writer::disk};
use std::sync::Arc;

pub(crate) fn candidate(
    broker: &FsVaultBroker,
    envelope: &crate::recorded::store::Envelope,
    row: events::Row,
) -> Result<ContentAuditCandidate, BrokerError> {
    Ok(ContentAuditCandidate {
        instance: Arc::clone(&broker.capture_instance),
        record_id: events::name(envelope, row.event.phase),
        digest: events::digest(&row)?,
        event: row.event,
    })
}
impl FsVaultBroker {
    /// 扫描至多 4096 个生命周期，每次返回至多 64 个待投事件，不返回部分坏批次。
    pub fn pending_content_audit(
        &self,
        limit: usize,
    ) -> Result<Vec<ContentAuditCandidate>, BrokerError> {
        if !(1..=64).contains(&limit) {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let names = events::entries(self)?;
        let mut candidates = Vec::new();
        for filename in &names {
            let (envelope, intent) = events::lifecycle(self, filename)?;
            let intent_ack = events::acknowledged(self, &envelope, &intent)?;
            let outcome = events::outcome_row(self, &envelope)?;
            let selected = if !intent_ack { Some(intent) } else { outcome };
            // 已 ACK 的 Outcome 也可重投以重试尚未完成的安全清理；不再执行正文写入。
            if candidates.len() < limit
                && let Some(row) = selected
            {
                candidates.push(candidate(self, &envelope, row)?);
            }
        }
        if events::entries(self)? != names {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(candidates)
    }
    /// owner 须先核对完整 AuditRecord；offset/hash 只是受信确认数据，不能单独认证账本。
    pub fn confirm_content_audit(
        &mut self,
        candidate: ContentAuditCandidate,
        offset: u64,
        record_hash: &str,
    ) -> Result<(), BrokerError> {
        if !Arc::ptr_eq(&candidate.instance, &self.capture_instance)
            || offset == 0
            || !disk::hex(record_hash)
        {
            return Err(BrokerError::CaptureChanged);
        }
        let id = candidate
            .event
            .event_id
            .strip_prefix("content-")
            .ok_or(BrokerError::CorruptWriterState)?;
        let envelope =
            crate::recorded::store::load_id(self, id)?.ok_or(BrokerError::CorruptWriterState)?;
        let intent = events::row(&envelope, None)?;
        let row = match candidate.event.phase {
            ContentAuditPhase::Intent => intent.clone(),
            ContentAuditPhase::Outcome => {
                if !events::acknowledged(self, &envelope, &intent)? {
                    return Err(BrokerError::CorruptWriterState);
                }
                events::outcome_row(self, &envelope)?.ok_or(BrokerError::CorruptWriterState)?
            }
        };
        if row.event != candidate.event
            || events::digest(&row)? != candidate.digest
            || events::name(&envelope, row.event.phase) != candidate.record_id
        {
            return Err(BrokerError::CaptureChanged);
        }
        let ack = events::Ack {
            version: 1,
            pair_id: self.writer.binding.pair_id,
            event_id: candidate.event.event_id.clone(),
            phase: candidate.event.phase,
            digest: candidate.digest,
            offset,
            record_hash: record_hash.to_owned(),
        };
        events::validate_ack(self, &row, &ack)?;
        let ack_path = format!("requests/{}.ack", candidate.record_id);
        let old = store::read(self, &ack_path, disk::RECORD_LIMIT)?
            .map(|bytes| disk::decode::<events::Ack>(&bytes))
            .transpose()?;
        if old.as_ref().is_some_and(|value| value != &ack) {
            return Err(BrokerError::CorruptWriterState);
        }
        let marker = format!(
            "outbox/{}",
            events::name(&envelope, ContentAuditPhase::Intent)
        );
        let exists = store::read(self, &marker, disk::RECORD_LIMIT)?.is_some();
        if exists {
            events::lifecycle(self, &events::name(&envelope, ContentAuditPhase::Intent))?;
        } else if old.is_none() {
            return Err(BrokerError::CorruptWriterState);
        }
        if old.is_none() {
            store::save(self, &ack_path, &ack, disk::RECORD_LIMIT, true)?;
        }
        crate::mutation::fault(self, 13)?;
        if candidate.event.phase == ContentAuditPhase::Outcome && exists {
            store::remove(self, &marker)?;
        }
        Ok(())
    }
}
