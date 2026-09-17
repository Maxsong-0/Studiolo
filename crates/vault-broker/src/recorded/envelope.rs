//! 不可变的受控元数据与原请求指纹，正文和原 key 不进入旁路记录。
use super::{MutationActor, MutationKind, MutationMetadata, store::id};
use crate::{
    BrokerError, FsVaultBroker, Mutation, Precondition, Revision, VaultPath, VaultUuid, sha256,
    writer::disk,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordingRef {
    pub id: String,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    pub version: u32,
    pub pair_id: VaultUuid,
    pub vault_uuid: VaultUuid,
    pub id: String,
    pub key_hash: String,
    pub fingerprint: String,
    pub path: VaultPath,
    pub precondition: Precondition,
    pub target: Revision,
    pub sha256: String,
    pub bytes: u64,
    pub requested_at_ms: Option<i64>,
    pub actor: Option<MutationActor>,
    pub kind: Option<MutationKind>,
}

impl Envelope {
    pub fn reference(&self) -> Result<RecordingRef, BrokerError> {
        Ok(RecordingRef {
            id: self.id.clone(),
            digest: sha256(&encode(self)?),
        })
    }
    pub fn validate(&self, broker: &FsVaultBroker) -> Result<(), BrokerError> {
        if !matches!(self.version, 1 | 2)
            || self.pair_id != broker.writer.binding.pair_id
            || self.vault_uuid != broker.vault_uuid
            || self.id != id(broker, &self.key_hash)
            || !disk::hex(&self.key_hash)
            || !disk::hex(&self.fingerprint)
            || !disk::hex(&self.sha256)
            || self.path.is_protected()
            || self.bytes
                > (if self.version == 2 {
                    crate::blob::OBJECT_LIMIT
                } else {
                    disk::CONTENT_LIMIT as u64
                })
            || self.requested_at_ms.is_some_and(|value| value < 0)
            || self.actor.is_some() != self.kind.is_some()
        {
            return Err(BrokerError::CorruptWriterState);
        }
        if self.version == 2 {
            crate::stream::validate_envelope(self)?;
        }
        if let (Some(actor), Some(kind)) = (&self.actor, &self.kind) {
            MutationMetadata::new(actor.clone(), kind.clone())
                .map_err(|_| BrokerError::CorruptWriterState)?;
            if matches!(kind, MutationKind::Restore { source_revision } if *source_revision != self.target)
            {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        Ok(())
    }
    pub fn matches(
        &self,
        request: &Mutation,
        metadata: Option<&MutationMetadata>,
    ) -> Result<(), BrokerError> {
        if self.version != 1
            || self.path != request.path
            || self.precondition != request.precondition
            || self.key_hash != sha256(request.idempotency_key.as_bytes())
            || self.fingerprint != FsVaultBroker::fingerprint(request)?
            || self.sha256 != sha256(&request.content)
            || self.bytes != request.content.len() as u64
            || metadata.is_some_and(|value| {
                self.actor.as_ref() != Some(&value.actor) || self.kind.as_ref() != Some(&value.kind)
            })
        {
            return Err(BrokerError::IdempotencyConflict);
        }
        Ok(())
    }
}

pub(crate) fn encode(envelope: &Envelope) -> Result<Vec<u8>, BrokerError> {
    serde_json::to_vec(envelope).map_err(|_| BrokerError::CorruptWriterState)
}
