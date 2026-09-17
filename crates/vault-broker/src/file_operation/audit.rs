//! 固定意图/永久结果各自确认；ACK 不充当认证，也不重新解释当前正文。
use super::{
    FileOperationAuditCandidate, FileOperationAuditEvent, FileOperationTicket,
    disk::{self, Folder},
    request::Request,
};
use crate::{BrokerError, ContentAuditPhase, FsVaultBroker, mutation, writer::disk as codec};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ack {
    version: u32,
    event_digest: String,
    offset: u64,
    hash: String,
}
pub(crate) fn intent(request: &Request) -> FileOperationAuditEvent {
    let f = &request.frozen;
    FileOperationAuditEvent {
        schema_version: f.version,
        event_id: crate::sha256(
            format!(
                "file-operation-event-v{}:{}:{}",
                f.version, f.operation_id, f.fingerprint
            )
            .as_bytes(),
        ),
        operation_id: f.operation_id.clone(),
        phase: ContentAuditPhase::Intent,
        vault_uuid: f.vault_uuid,
        actor: f.actor.clone(),
        path: f.path.clone(),
        kind: f.kind.clone(),
        scope_hash: f.scope_hash.clone(),
        scope_epoch: f.scope_epoch,
        requested_at_ms: f.requested_at_ms,
        request_fingerprint: f.fingerprint.clone(),
        outcome: None,
        effects: Vec::new(),
        tombstone_id: None,
        evidence_id: None,
    }
}
pub(crate) fn outcome(
    request: &Request,
    terminal: &super::record::Terminal,
) -> Result<FileOperationAuditEvent, BrokerError> {
    if terminal.frozen != request.frozen
        || terminal.receipt.sequence != request.sequence
        || terminal.observed_before != request.observed_before
        || terminal.rename.as_ref().map(|r| &r.before) != request.secondary.as_ref()
    {
        return Err(BrokerError::CorruptWriterState);
    }
    let mut event = intent(request);
    event.phase = ContentAuditPhase::Outcome;
    event.outcome = Some(terminal.receipt.outcome);
    event.effects = terminal.receipt.effects.clone();
    event.tombstone_id = terminal.receipt.tombstone_id.clone();
    event.evidence_id = terminal.receipt.evidence_id.clone();
    Ok(event)
}
fn filename(event: &FileOperationAuditEvent) -> String {
    let phase = match event.phase {
        ContentAuditPhase::Intent => "intent",
        ContentAuditPhase::Outcome => "outcome",
    };
    format!("{}-{phase}.json", event.operation_id)
}
pub(crate) fn confirmed(
    broker: &FsVaultBroker,
    event: &FileOperationAuditEvent,
) -> Result<bool, BrokerError> {
    let Some(bytes) = disk::read(broker, Folder::Acks, &filename(event))? else {
        return Ok(false);
    };
    let value: Ack = codec::decode(&bytes)?;
    if value.version != 1
        || value.offset == 0
        || !codec::hex(&value.hash)
        || value.event_digest != super::digest(event)?
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(true)
}
impl FsVaultBroker {
    /// 仅确认可信 owner 已验证的完整实际 Intent 回执，不开始文件操作。
    pub fn confirm_file_operation_intent(
        &mut self,
        ticket: &FileOperationTicket,
        offset: u64,
        hash: &str,
    ) -> Result<(), BrokerError> {
        super::ticket::check(self, ticket)?;
        self.confirm_file_operation_audit(
            FileOperationAuditCandidate {
                instance: Arc::clone(&self.capture_instance),
                event: ticket.intent.clone(),
            },
            offset,
            hash,
        )
    }
    /// 只返回当前单活动槽最多两个固定事件；不会开始尚未执行的决定。
    pub fn pending_file_operation_audit(
        &self,
        limit: usize,
    ) -> Result<Vec<FileOperationAuditCandidate>, BrokerError> {
        if !(1..=64).contains(&limit) {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let state = mutation::load(self)?;
        let Some(active) = &state.file_operation else {
            return Ok(Vec::new());
        };
        let request = super::request::ensure(self, active)?;
        let mut events = vec![intent(&request)];
        if let Some(terminal) = &active.terminal {
            events.push(outcome(&request, terminal)?);
        }
        let mut result = Vec::new();
        for event in events {
            if !confirmed(self, &event)? && result.len() < limit {
                result.push(FileOperationAuditCandidate {
                    instance: Arc::clone(&self.capture_instance),
                    event,
                });
            }
        }
        Ok(result)
    }
    /// 消费同实例固定候选；Outcome ACK 只关闭已存在的原终态。
    pub fn confirm_file_operation_audit(
        &mut self,
        candidate: FileOperationAuditCandidate,
        offset: u64,
        hash: &str,
    ) -> Result<(), BrokerError> {
        if !Arc::ptr_eq(&self.capture_instance, &candidate.instance) {
            return Err(BrokerError::CaptureChanged);
        }
        if offset == 0 || !codec::hex(hash) {
            return Err(BrokerError::IdempotencyConflict);
        }
        let mut state = mutation::load(self)?;
        let request = super::request::load(self, &candidate.event.operation_id)?
            .ok_or(BrokerError::CaptureChanged)?;
        let initial = intent(&request);
        let expected = match candidate.event.phase {
            ContentAuditPhase::Intent => initial.clone(),
            ContentAuditPhase::Outcome => {
                if !confirmed(self, &initial)? {
                    return Err(BrokerError::MutationUnresolved);
                }
                let terminal = super::terminal::load(self, &request.frozen)?
                    .ok_or(BrokerError::MutationUnresolved)?;
                outcome(&request, &terminal)?
            }
        };
        if expected != candidate.event {
            return Err(BrokerError::CaptureChanged);
        }
        let ack = Ack {
            version: 1,
            event_digest: super::digest(&expected)?,
            offset,
            hash: hash.to_owned(),
        };
        disk::save(self, Folder::Acks, &filename(&expected), &ack, None)?;
        mutation::fault(self, 59)?;
        super::finish::resume(self, &mut state)
    }
}
