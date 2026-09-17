//! Completed 的历史索引收尾是幂等的，先保存可复核历史再公开水位。
use super::{
    HistoryEntry,
    catalog::{Item, load, save},
    store,
};
use crate::mutation::record::{Operation, Phase};
use crate::{BrokerError, FsVaultBroker, mutation, sha256};
pub(crate) fn complete(broker: &FsVaultBroker, op: &Operation) -> Result<(), BrokerError> {
    let Some(reference) = &op.recording else {
        return Ok(());
    };
    if op.phase != Phase::Completed {
        return Err(BrokerError::CorruptWriterState);
    }
    let envelope = crate::recorded::store::load(broker, reference)?;
    if envelope.path != op.path
        || envelope.target != op.target_revision
        || envelope.fingerprint != op.fingerprint
    {
        return Err(BrokerError::CorruptWriterState);
    }
    let mut catalog = load(broker, &op.path)?;
    let id = sha256(format!("{}:new", envelope.id).as_bytes());
    let already = catalog.entries.iter().any(|item| item.entry.id == id);
    let original_len = catalog.entries.len();
    let at = store::now();
    if let Some(previous) = op.previous_revision {
        let previous_object = super::blob::object(broker, &op.path, previous)?;
        if !catalog
            .entries
            .iter()
            .any(|item| item.entry.revision == previous)
        {
            catalog.entries.push(Item {
                sequence: op.sequence,
                recording: Some(reference.clone()),
                file_operation_id: None,
                entry: HistoryEntry {
                    id: sha256(format!("{}:previous", envelope.id).as_bytes()),
                    revision: previous,
                    bytes: previous_object.identity.bytes,
                    authored_at_ms: None,
                    author: None,
                    archived_at_ms: at,
                    archived_by: envelope.actor.clone(),
                },
            });
        }
    }
    let target = super::blob::object(broker, &op.path, op.target_revision)?;
    if target.identity.bytes != envelope.bytes || target.identity.sha256 != envelope.sha256 {
        return Err(BrokerError::CorruptWriterState);
    }
    if already {
        if catalog.entries.len() != original_len {
            return Err(BrokerError::CorruptWriterState);
        }
        return Ok(());
    }
    catalog.entries.push(Item {
        sequence: op.sequence,
        recording: Some(reference.clone()),
        file_operation_id: None,
        entry: HistoryEntry {
            id,
            revision: op.target_revision,
            bytes: envelope.bytes,
            authored_at_ms: envelope.requested_at_ms,
            author: envelope.actor.clone(),
            archived_at_ms: at,
            archived_by: envelope.actor,
        },
    });
    save(broker, &mut catalog)?;
    mutation::fault(broker, 14)
}
