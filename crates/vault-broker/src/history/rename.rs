//! 同一父终态投影到两个独立路径 catalog；中断后按角色 ID 幂等补缺，不再改变文件。
use super::{
    HistoryEntry,
    catalog::{self, Item},
};
use crate::{
    BrokerError, FileEffect, FileOperationKind, FileOperationOutcome, FilePresence, FsVaultBroker,
    VaultPath,
    file_operation::record::{Active, Terminal},
};
fn bad() -> BrokerError {
    BrokerError::CorruptWriterState
}
fn entry(t: &Terminal, path: &VaultPath) -> Result<HistoryEntry, BrokerError> {
    let FileOperationKind::Rename { to, .. } = &t.frozen.kind else {
        return Err(bad());
    };
    let (presence, role, time) = if path == &t.frozen.path {
        (&t.observed_before, "source", t.effect_at_ms)
    } else if path == to {
        let Some(FileEffect::Confirmed { after, .. }) = t.receipt.effects.first() else {
            return Err(bad());
        };
        (
            after,
            "target",
            t.rename.as_ref().ok_or_else(bad)?.target_effect_at_ms,
        )
    } else {
        return Err(bad());
    };
    let FilePresence::Present {
        revision, bytes, ..
    } = presence
    else {
        return Err(bad());
    };
    Ok(HistoryEntry {
        id: crate::sha256(
            format!("file-operation-history-v2:{}:{role}", t.frozen.operation_id).as_bytes(),
        ),
        revision: *revision,
        bytes: *bytes,
        authored_at_ms: None,
        author: None,
        archived_at_ms: time,
        archived_by: Some(t.frozen.actor.clone()),
    })
}
pub(super) fn complete(broker: &FsVaultBroker, a: &Active) -> Result<(), BrokerError> {
    let t = a.terminal.as_ref().ok_or_else(bad)?;
    if t.receipt.outcome != FileOperationOutcome::Committed {
        return Err(bad());
    }
    let (to, r) = crate::file_operation::rename_model::get(a)?;
    for (index, (path, proof)) in [
        (&a.frozen.path, a.history.as_ref()),
        (to, r.history.as_ref()),
    ]
    .into_iter()
    .enumerate()
    {
        let entry = entry(t, path)?;
        let actual = super::blob::object(broker, path, entry.revision)?;
        if actual.identity != proof.ok_or_else(bad)?.identity {
            return Err(bad());
        }
        let item = Item {
            sequence: a.sequence,
            entry,
            recording: None,
            file_operation_id: Some(a.frozen.operation_id.clone()),
        };
        let mut catalog = catalog::load(broker, path)?;
        if let Some(old) = catalog.entries.iter().find(|e| e.entry.id == item.entry.id) {
            if old.entry != item.entry
                || old.sequence != item.sequence
                || old.recording.is_some()
                || old.file_operation_id != item.file_operation_id
            {
                return Err(bad());
            }
        } else {
            catalog.version = 3;
            catalog.entries.push(item);
            catalog::save(broker, &mut catalog)?;
        }
        crate::mutation::fault(broker, if index == 0 { 80 } else { 81 })?;
    }
    Ok(())
}
pub(super) fn validate(path: &VaultPath, item: &Item, t: &Terminal) -> Result<(), BrokerError> {
    if item.sequence != t.receipt.sequence
        || item.recording.is_some()
        || t.receipt.outcome != FileOperationOutcome::Committed
        || item.file_operation_id.as_ref() != Some(&t.frozen.operation_id)
        || item.entry != entry(t, path)?
    {
        return Err(bad());
    }
    Ok(())
}
