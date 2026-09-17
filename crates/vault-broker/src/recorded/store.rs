//! 请求描述符独立保留以支撑历史正文清理后的恢复幂等；日志只引用精确摘要。

use super::MutationMetadata;
use super::envelope::encode;
pub(crate) use super::envelope::{Envelope, RecordingRef};
use crate::history::store;
use crate::writer::disk;
use crate::{BrokerError, FsVaultBroker, Mutation, Revision, sha256};

pub(crate) fn id(broker: &FsVaultBroker, key_hash: &str) -> String {
    sha256(
        format!(
            "studiolo-content-v1:{}:{key_hash}",
            broker.writer.binding.pair_id
        )
        .as_bytes(),
    )
}

pub(crate) fn load_id(broker: &FsVaultBroker, id: &str) -> Result<Option<Envelope>, BrokerError> {
    if !disk::hex(id) {
        return Err(BrokerError::CorruptWriterState);
    }
    let Some(bytes) = store::read(broker, &format!("requests/{id}.json"), disk::RECORD_LIMIT)?
    else {
        return Ok(None);
    };
    let envelope: Envelope = disk::decode(&bytes)?;
    envelope.validate(broker)?;
    if envelope.id != id || encode(&envelope)? != bytes {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(Some(envelope))
}
pub(crate) fn load(
    broker: &FsVaultBroker,
    reference: &RecordingRef,
) -> Result<Envelope, BrokerError> {
    let envelope = load_id(broker, &reference.id)?.ok_or(BrokerError::CorruptWriterState)?;
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
