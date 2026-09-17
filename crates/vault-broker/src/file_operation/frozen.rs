//! 原键只保存摘要；完整请求绑定独立于旧 Mutation v1 指纹，不改旧算法。
use super::{
    audit_model::FileOperationKind,
    limits::FileOperationLimits,
    model::{FileOperationMetadata, FileOperationRequest, FileOperationScopeHash},
};
use crate::{BrokerError, FsVaultBroker, MutationActor, VaultPath, VaultUuid, writer::disk};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Frozen {
    pub version: u32,
    pub operation_id: String,
    pub key_hash: String,
    pub fingerprint: String,
    pub vault_uuid: VaultUuid,
    pub pair_id: VaultUuid,
    pub path: VaultPath,
    pub kind: FileOperationKind,
    pub actor: MutationActor,
    pub scope_hash: FileOperationScopeHash,
    pub scope_epoch: u64,
    pub retention: String,
    pub limits: FileOperationLimits,
    pub requested_at_ms: Option<i64>,
}

impl Frozen {
    pub(crate) fn new(
        broker: &FsVaultBroker,
        request: &FileOperationRequest,
        metadata: &FileOperationMetadata,
        limits: FileOperationLimits,
    ) -> Result<Self, BrokerError> {
        limits.validate()?;
        let (path, key, kind) = parts(request);
        validate_path(path)?;
        if let FileOperationKind::Rename { to, .. } = &kind {
            super::rename_model::paths(path, to)?;
        }
        if key.is_empty() {
            return Err(BrokerError::IdempotencyConflict);
        }
        if key.len() > 4096 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        if let FileOperationKind::RestoreTombstone { tombstone_id } = &kind
            && !disk::hex(tombstone_id)
        {
            return Err(BrokerError::IdempotencyConflict);
        }
        FileOperationMetadata::new(
            metadata.actor.clone(),
            metadata.scope_hash.clone(),
            metadata.scope_epoch,
        )?;
        let key_hash = crate::sha256(key.as_bytes());
        let operation_id = operation_id(broker.writer.binding.pair_id, &key_hash);
        let mut result = Self {
            version: if matches!(kind, FileOperationKind::Rename { .. }) {
                2
            } else {
                1
            },
            operation_id,
            key_hash,
            fingerprint: String::new(),
            vault_uuid: broker.vault_uuid,
            pair_id: broker.writer.binding.pair_id,
            path: path.clone(),
            kind,
            actor: metadata.actor.clone(),
            scope_hash: metadata.scope_hash.clone(),
            scope_epoch: metadata.scope_epoch,
            retention: "full".into(),
            limits,
            requested_at_ms: crate::history::store::now(),
        };
        result.fingerprint = result.digest()?;
        Ok(result)
    }
    pub(crate) fn validate(&self, pair: VaultUuid) -> Result<(), BrokerError> {
        self.limits.validate()?;
        validate_path(&self.path)?;
        FileOperationMetadata::new(
            self.actor.clone(),
            self.scope_hash.clone(),
            self.scope_epoch,
        )?;
        if let FileOperationKind::Rename { to, .. } = &self.kind {
            super::rename_model::paths(&self.path, to)?;
        }
        if self.version
            != if matches!(self.kind, FileOperationKind::Rename { .. }) {
                2
            } else {
                1
            }
            || self.pair_id != pair
            || !disk::hex(&self.key_hash)
            || self.operation_id != operation_id(pair, &self.key_hash)
            || self.retention != "full"
            || self.requested_at_ms.is_some_and(|v| v < 0)
            || self.fingerprint != self.digest()?
        {
            return Err(BrokerError::CorruptWriterState);
        }
        if let FileOperationKind::RestoreTombstone { tombstone_id } = &self.kind
            && !disk::hex(tombstone_id)
        {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(())
    }
    pub(crate) fn matches(&self, other: &Self) -> bool {
        self.operation_id == other.operation_id
            && self.key_hash == other.key_hash
            && self.fingerprint == other.fingerprint
    }
    fn digest(&self) -> Result<String, BrokerError> {
        let bytes = serde_json::to_vec(&(
            if self.version == 2 {
                "studiolo-file-operation-v2"
            } else {
                "studiolo-file-operation-v1"
            },
            self.vault_uuid,
            self.pair_id,
            &self.key_hash,
            &self.path,
            &self.kind,
            &self.actor,
            &self.scope_hash,
            self.scope_epoch,
            &self.retention,
            self.limits,
        ))
        .map_err(|_| BrokerError::CorruptWriterState)?;
        Ok(crate::sha256(&bytes))
    }
}

pub(crate) fn parts(request: &FileOperationRequest) -> (&VaultPath, &str, FileOperationKind) {
    match request {
        FileOperationRequest::Rename {
            from,
            expected,
            to,
            idempotency_key,
        } => (
            from,
            idempotency_key,
            FileOperationKind::Rename {
                expected: *expected,
                to: to.clone(),
            },
        ),
        FileOperationRequest::Delete {
            path,
            expected,
            idempotency_key,
        } => (
            path,
            idempotency_key,
            FileOperationKind::Delete {
                expected: *expected,
            },
        ),
        FileOperationRequest::RestoreTombstone {
            path,
            tombstone_id,
            idempotency_key,
        } => (
            path,
            idempotency_key,
            FileOperationKind::RestoreTombstone {
                tombstone_id: tombstone_id.clone(),
            },
        ),
    }
}
pub(crate) fn operation_id(pair: VaultUuid, key_hash: &str) -> String {
    crate::sha256(format!("studiolo-file-operation-id-v1:{pair}:{key_hash}").as_bytes())
}
pub(crate) fn validate_path(path: &VaultPath) -> Result<(), BrokerError> {
    crate::blob::file::path_budget(path)?;
    if path.is_protected() || path.as_str().split('/').any(|part| part.starts_with('.')) {
        return Err(BrokerError::ProtectedPath(path.clone()));
    }
    Ok(())
}
