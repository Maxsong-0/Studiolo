//! 请求描述符独立保留以支撑历史正文清理后的恢复幂等；日志只引用精确摘要。

use super::MutationMetadata;
use super::envelope::encode;
pub(crate) use super::envelope::{Envelope, RecordingRef};
use crate::history::store;
use crate::writer::disk;
use crate::{BrokerError, FsVaultBroker, Mutation, Revision, records::WriterRecords, sha256};

pub(crate) fn id(records: &dyn WriterRecords, key_hash: &str) -> String {
    sha256(format!("studiolo-content-v1:{}:{key_hash}", records.pair_id()).as_bytes())
}

pub(crate) fn load_id(
    records: &dyn WriterRecords,
    id: &str,
) -> Result<Option<Envelope>, BrokerError> {
    if !disk::hex(id) {
        return Err(BrokerError::CorruptWriterState);
    }
    store::read(records, &format!("requests/{id}.json"), disk::RECORD_LIMIT)?
        .map(|bytes| decode_id(records, id, &bytes))
        .transpose()
}
/// 描述符原字节 → 核过的描述符；文件名、库和编码都要对得上。
pub(crate) fn decode_id(
    records: &dyn WriterRecords,
    id: &str,
    bytes: &[u8],
) -> Result<Envelope, BrokerError> {
    let envelope: Envelope = disk::decode(bytes)?;
    envelope.validate(records)?;
    if envelope.id != id || encode(&envelope)? != bytes {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(envelope)
}
pub(crate) fn load(
    records: &dyn WriterRecords,
    reference: &RecordingRef,
) -> Result<Envelope, BrokerError> {
    let envelope = load_id(records, &reference.id)?.ok_or(BrokerError::CorruptWriterState)?;
    if envelope.reference()? != *reference {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(envelope)
}

pub(crate) fn prepare(
    broker: &FsVaultBroker,
    request: &Mutation,
    metadata: Option<&MutationMetadata>,
) -> Result<Envelope, BrokerError> {
    let key_hash = sha256(request.idempotency_key.as_bytes());
    let id = id(broker, &key_hash);
    let envelope = match load_id(broker, &id)? {
        Some(envelope) => {
            envelope.matches(request, metadata)?;
            envelope
        }
        None => {
            let envelope = Envelope {
                version: 1,
                pair_id: broker.writer.binding.pair_id,
                vault_uuid: broker.vault_uuid,
                id: id.clone(),
                key_hash,
                fingerprint: FsVaultBroker::fingerprint(request)?,
                path: request.path.clone(),
                precondition: request.precondition,
                target: Revision::compute(broker.vault_uuid, &request.path, &request.content),
                sha256: sha256(&request.content),
                bytes: request.content.len() as u64,
                requested_at_ms: store::now(),
                actor: metadata.map(|value| value.actor.clone()),
                kind: metadata.map(|value| value.kind.clone()),
            };
            envelope.validate(broker)?;
            super::capacity::reserve(broker, encode(&envelope)?.len())?;
            store::save(
                broker,
                &format!("requests/{id}.json"),
                &envelope,
                disk::RECORD_LIMIT,
                true,
            )?;
            envelope
        }
    };
    crate::content_audit::stage_intent(broker, &envelope)?;
    Ok(envelope)
}
