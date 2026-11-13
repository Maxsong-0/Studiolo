//! 不信feed里的作者或revision副本；用已验证历史引用重建操作语义。
use super::super::catalog::Item;
use super::{ChangeEntry, ChangeKind};
use crate::{
    BrokerError, FileEffect, FileOperationKind, FilePresence, FsVaultBroker, Precondition,
    VaultPath,
};
fn bad() -> BrokerError {
    BrokerError::CorruptWriterState
}
fn revision(p: &FilePresence) -> Option<crate::Revision> {
    match p {
        FilePresence::Missing => None,
        FilePresence::Present { revision, .. } => Some(*revision),
    }
}
fn bytes(p: &FilePresence) -> u64 {
    match p {
        FilePresence::Missing => 0,
        FilePresence::Present { bytes, .. } => *bytes,
    }
}
pub(super) fn project(
    broker: &FsVaultBroker,
    path: &VaultPath,
    item: &Item,
) -> Result<ChangeEntry, BrokerError> {
    let mut row = ChangeEntry {
        id: item.entry.id.clone(),
        sequence: item.sequence,
        path: path.clone(),
        kind: ChangeKind::Replaced,
        actor: None,
        previous_revision: None,
        revision: None,
        bytes: 0,
        at_ms: None,
        counterpart: None,
    };
    if let Some(reference) = &item.recording {
        let e = crate::recorded::store::load(broker, reference)?;
        if item.entry.id != crate::sha256(format!("{}:new", e.id).as_bytes()) {
            return Err(bad());
        }
        row.kind = if matches!(e.kind, Some(crate::MutationKind::Restore { .. })) {
            ChangeKind::Restored
        } else if e.precondition == Precondition::MustNotExist {
            ChangeKind::Created
        } else {
            ChangeKind::Replaced
        };
        row.actor = e.actor;
        row.at_ms = e.requested_at_ms;
        row.previous_revision = match e.precondition {
            Precondition::MustNotExist => None,
            Precondition::Match(r) => Some(r),
        };
        row.revision = Some(e.target);
        row.bytes = e.bytes;
    } else if let Some(reference) = &item.batch {
        let (active, _) = crate::batch::record::history_source(
            broker,
            &reference.batch_id,
            reference.sequence,
            reference.rollback,
        )?;
        let op = active
            .items
            .iter()
            .find(|op| &op.path == path)
            .ok_or_else(bad)?;
        row.kind = if active.rollback {
            ChangeKind::BatchRollback
        } else {
            ChangeKind::Batch
        };
        row.actor = active.metadata.as_ref().map(|m| m.actor.clone());
        row.at_ms = active.metadata.as_ref().and_then(|m| m.requested_at_ms);
        (row.previous_revision, row.revision) = if active.rollback {
            (Some(op.target_revision), op.previous_revision)
        } else {
            (op.previous_revision, Some(op.target_revision))
        };
        row.bytes = if row.revision.is_some() {
            item.entry.bytes
        } else {
            0
        };
        if row.revision.is_some_and(|r| r != item.entry.revision) {
            return Err(bad());
        }
    } else if let Some(id) = &item.file_operation_id {
        let request = crate::file_operation::request::load(broker, id)?.ok_or_else(bad)?;
        let terminal =
            crate::file_operation::terminal::load(broker, &request.frozen)?.ok_or_else(bad)?;
        row.actor = Some(terminal.frozen.actor.clone());
        row.at_ms = terminal.frozen.requested_at_ms;
        let Some(FileEffect::Confirmed { after, .. }) = terminal.receipt.effects.first() else {
            return Err(bad());
        };
        match &terminal.frozen.kind {
            FileOperationKind::Delete { .. } => {
                row.kind = ChangeKind::Deleted;
                row.previous_revision = revision(&terminal.observed_before);
            }
            FileOperationKind::RestoreTombstone { .. } => {
                row.kind = ChangeKind::Restored;
                row.revision = revision(after);
                row.bytes = bytes(after);
            }
            FileOperationKind::Rename { to, .. } => {
                row.kind = ChangeKind::Renamed;
                if path == &terminal.frozen.path {
                    row.previous_revision = revision(&terminal.observed_before);
                    row.counterpart = Some(to.clone());
                } else if path == to {
                    row.revision = revision(after);
                    row.bytes = bytes(after);
                    row.counterpart = Some(terminal.frozen.path.clone());
                } else {
                    return Err(bad());
                }
            }
        }
    } else {
        return Err(bad());
    }
    Ok(row)
}
