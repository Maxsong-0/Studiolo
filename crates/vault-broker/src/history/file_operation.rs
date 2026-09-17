//! typed 历史来源绑定原持久效果；删除旧作者未知，恢复是新 CAS 提交而非旧操作重放。
use super::{
    HistoryEntry,
    catalog::{self, Item},
};
use crate::file_operation::{
    record::{Active, Terminal},
    request, terminal,
};
use crate::{
    BrokerError, FileEffect, FileOperationKind, FileOperationOutcome, FilePresence, FsVaultBroker,
    VaultPath,
};

fn entry(value: &Terminal) -> Result<HistoryEntry, BrokerError> {
    let frozen = &value.frozen;
    let (presence, restored) = match &frozen.kind {
        FileOperationKind::Rename { .. } => return Err(BrokerError::CorruptWriterState),
        FileOperationKind::Delete { .. } => (&value.observed_before, false),
        FileOperationKind::RestoreTombstone { .. } => {
            let Some(FileEffect::Confirmed { after, .. }) = value.receipt.effects.first() else {
                return Err(BrokerError::CorruptWriterState);
            };
            (after, true)
        }
    };
    let FilePresence::Present {
        revision, bytes, ..
    } = presence
    else {
        return Err(BrokerError::CorruptWriterState);
    };
    Ok(HistoryEntry {
        id: crate::sha256(format!("file-operation-history-v1:{}", frozen.operation_id).as_bytes()),
        revision: *revision,
        bytes: *bytes,
        authored_at_ms: if restored {
            frozen.requested_at_ms
        } else {
            None
        },
        author: restored.then(|| frozen.actor.clone()),
        archived_at_ms: value.effect_at_ms,
        archived_by: Some(frozen.actor.clone()),
    })
}
pub(crate) fn complete(broker: &FsVaultBroker, active: &Active) -> Result<(), BrokerError> {
    if active.rename.is_some() {
        return super::rename::complete(broker, active);
    }
    let terminal = active
        .terminal
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    if terminal.receipt.outcome != FileOperationOutcome::Committed {
        return Err(BrokerError::CorruptWriterState);
    }
    let entry = entry(terminal)?;
    let object = super::blob::object(broker, &active.frozen.path, entry.revision)?;
    let expected = active
        .history
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    if object.identity != expected.identity {
        return Err(BrokerError::CorruptWriterState);
    }
    let item = Item {
        sequence: active.sequence,
        entry,
        recording: None,
        file_operation_id: Some(active.frozen.operation_id.clone()),
    };
    let mut catalog = catalog::load(broker, &active.frozen.path)?;
    if let Some(existing) = catalog.entries.iter().find(|v| v.entry.id == item.entry.id) {
        return if existing.sequence == item.sequence
            && existing.entry == item.entry
            && existing.recording.is_none()
            && existing.file_operation_id == item.file_operation_id
        {
            Ok(())
        } else {
            Err(BrokerError::CorruptWriterState)
        };
    }
    catalog.version = 3;
    catalog.entries.push(item);
    catalog::save(broker, &mut catalog)
}
pub(super) fn validate(
    broker: &FsVaultBroker,
    path: &VaultPath,
    item: &Item,
) -> Result<(), BrokerError> {
    let id = item
        .file_operation_id
        .as_deref()
        .ok_or(BrokerError::CorruptWriterState)?;
    if item.recording.is_some() {
        return Err(BrokerError::CorruptWriterState);
    }
    let request = request::load(broker, id)?.ok_or(BrokerError::CorruptWriterState)?;
    let terminal =
        terminal::load(broker, &request.frozen)?.ok_or(BrokerError::CorruptWriterState)?;
    if matches!(terminal.frozen.kind, FileOperationKind::Rename { .. }) {
        return super::rename::validate(path, item, &terminal);
    }
    if &terminal.frozen.path != path
        || terminal.receipt.sequence != item.sequence
        || terminal.receipt.outcome != FileOperationOutcome::Committed
        || entry(&terminal)? != item.entry
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(())
}
