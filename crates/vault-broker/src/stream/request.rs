//! 新请求描述符只哈希受控元数据；v1 含正文指纹永不重算或迁移。
use super::StreamMutation;
use crate::recorded::envelope::{Envelope, encode};
use crate::{BrokerError, FsVaultBroker, MutationMetadata, Revision, recorded::store, sha256};

pub(super) fn validate(request: &StreamMutation) -> Result<(), BrokerError> {
    request.content.validate()?;
    crate::blob::file::path_budget(&request.path)?;
    if request.path.is_protected() {
        return Err(BrokerError::ProtectedPath(request.path.clone()));
    }
    if request.idempotency_key.is_empty() || request.idempotency_key.len() > 4096 {
        return Err(BrokerError::IdempotencyConflict);
    }
    Ok(())
}
pub(super) fn fingerprint(
    request: &StreamMutation,
    metadata: &MutationMetadata,
) -> Result<String, BrokerError> {
    fingerprint_parts(
        &request.path,
        request.precondition,
        &sha256(request.idempotency_key.as_bytes()),
        &request.content,
        &metadata.actor,
        &metadata.kind,
    )
}
fn fingerprint_parts(
    path: &crate::VaultPath,
    precondition: crate::Precondition,
    key_hash: &str,
    content: &crate::BlobIdentity,
    actor: &crate::MutationActor,
    kind: &crate::MutationKind,
) -> Result<String, BrokerError> {
    let bytes = serde_json::to_vec(&(
        "studiolo-stream-request-v2",
        path,
        precondition,
        key_hash,
        content,
        actor,
        kind,
    ))
    .map_err(|_| BrokerError::CorruptWriterState)?;
    Ok(sha256(&bytes))
}
pub(crate) fn validate_envelope(value: &Envelope) -> Result<(), BrokerError> {
    crate::blob::file::path_budget(&value.path).map_err(|_| BrokerError::CorruptWriterState)?;
    let actor = value
        .actor
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    let kind = value.kind.as_ref().ok_or(BrokerError::CorruptWriterState)?;
    let content = crate::BlobIdentity {
        bytes: value.bytes,
        sha256: value.sha256.clone(),
    };
    if fingerprint_parts(
        &value.path,
        value.precondition,
        &value.key_hash,
        &content,
        actor,
        kind,
    )? != value.fingerprint
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(())
}
pub(super) fn matches(
    envelope: &Envelope,
    request: &StreamMutation,
    metadata: &MutationMetadata,
) -> Result<(), BrokerError> {
    if envelope.version != 2
        || envelope.path != request.path
        || envelope.precondition != request.precondition
        || envelope.key_hash != sha256(request.idempotency_key.as_bytes())
        || envelope.fingerprint != fingerprint(request, metadata)?
        || envelope.bytes != request.content.bytes
        || envelope.sha256 != request.content.sha256
        || envelope.actor.as_ref() != Some(&metadata.actor)
        || envelope.kind.as_ref() != Some(&metadata.kind)
    {
        return Err(BrokerError::IdempotencyConflict);
    }
    Ok(())
}
pub(super) fn prepare(
    broker: &FsVaultBroker,
    request: &StreamMutation,
    target: Revision,
    metadata: &MutationMetadata,
) -> Result<Envelope, BrokerError> {
    let hash = sha256(request.idempotency_key.as_bytes());
    let id = store::id(broker, &hash);
    let value = match store::load_id(broker, &id)? {
        Some(value) => {
            matches(&value, request, metadata)?;
            if value.target != target {
                return Err(BrokerError::IdempotencyConflict);
            }
            value
        }
        None => {
            let value = Envelope {
                version: 2,
                pair_id: broker.writer.binding.pair_id,
                vault_uuid: broker.vault_uuid,
                id: id.clone(),
                key_hash: hash,
                fingerprint: fingerprint(request, metadata)?,
                path: request.path.clone(),
                precondition: request.precondition,
                target,
                sha256: request.content.sha256.clone(),
                bytes: request.content.bytes,
                requested_at_ms: crate::history::store::now(),
                actor: Some(metadata.actor.clone()),
                kind: Some(metadata.kind.clone()),
            };
            value.validate(broker)?;
            crate::recorded::capacity::reserve(broker, encode(&value)?.len())?;
            crate::history::store::save(
                broker,
                &format!("requests/{id}.json"),
                &value,
                crate::writer::disk::RECORD_LIMIT,
                true,
            )?;
            value
        }
    };
    crate::content_audit::stage_intent(broker, &value)?;
    Ok(value)
}
