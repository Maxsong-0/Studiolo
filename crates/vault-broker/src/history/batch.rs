//! 批量历史只从完成终态派生；原提交在后续整批回滚时不被重写。
use super::{
    HistoryEntry,
    catalog::{self, Item},
};
use crate::{
    BrokerError, FsVaultBroker, Revision, VaultPath, batch::record, mutation::record::Operation,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reference {
    pub batch_id: String,
    pub sequence: u64,
    pub rollback: bool,
}
fn bad() -> BrokerError {
    BrokerError::CorruptWriterState
}
fn id(active: &record::Active, path: &VaultPath, role: &str) -> String {
    crate::sha256(
        format!(
            "batch-history-v1:{}:{}:{}:{path}:{role}",
            active.batch_id, active.sequence, active.rollback
        )
        .as_bytes(),
    )
}
fn entry(
    broker: &FsVaultBroker,
    active: &record::Active,
    op: &Operation,
    revision: Revision,
    current: bool,
) -> Result<HistoryEntry, BrokerError> {
    let object = super::blob::object(broker, &op.path, revision)?;
    let actor = active.metadata.as_ref().map(|m| m.actor.clone());
    let at = active.metadata.as_ref().and_then(|m| m.requested_at_ms);
    Ok(HistoryEntry {
        id: id(active, &op.path, if current { "new" } else { "previous" }),
        revision,
        bytes: object.identity.bytes,
        authored_at_ms: if current { at } else { None },
        author: if current { actor.clone() } else { None },
        archived_at_ms: at,
        archived_by: actor,
    })
}
pub(crate) fn complete(broker: &FsVaultBroker, active: &record::Active) -> Result<(), BrokerError> {
    for op in &active.items {
        let mut catalog = catalog::load(broker, &op.path)?;
        let (before, after) = if active.rollback {
            (Some(op.target_revision), op.previous_revision)
        } else {
            (op.previous_revision, Some(op.target_revision))
        };
        let mut changed = false;
        for (revision, current) in before
            .into_iter()
            .map(|r| (r, false))
            .chain(after.map(|r| (r, true)))
        {
            // 普通归档只补未知旧版本；删除式回滚必须有本次独立的归档条目。
            if !current
                && after.is_some()
                && catalog.entries.iter().any(|i| i.entry.revision == revision)
            {
                continue;
            }
            let value = entry(broker, active, op, revision, current)?;
            let reference = Reference {
                batch_id: active.batch_id.clone(),
                sequence: active.sequence,
                rollback: active.rollback,
            };
            if let Some(old) = catalog.entries.iter().find(|i| i.entry.id == value.id) {
                if old.entry != value
                    || old.sequence != active.sequence
                    || old.batch.as_ref() != Some(&reference)
                    || old.recording.is_some()
                    || old.file_operation_id.is_some()
                {
                    return Err(bad());
                }
            } else {
                catalog.entries.push(Item {
                    sequence: active.sequence,
                    entry: value,
                    batch: Some(reference),
                    recording: None,
                    file_operation_id: None,
                });
                changed = true;
            }
        }
        if changed {
            catalog.version = 4;
            catalog::save(broker, &mut catalog)?;
        }
        crate::mutation::fault(broker, 45)?;
        let primary = id(
            active,
            &op.path,
            if after.is_some() { "new" } else { "previous" },
        );
        let item = catalog
            .entries
            .iter()
            .find(|i| i.entry.id == primary)
            .ok_or_else(bad)?;
        super::feed::record(broker, &op.path, item)?;
    }
    Ok(())
}
pub(super) fn validate(
    broker: &FsVaultBroker,
    path: &VaultPath,
    item: &Item,
) -> Result<(), BrokerError> {
    let r = item.batch.as_ref().ok_or_else(bad)?;
    if item.recording.is_some() || item.file_operation_id.is_some() || item.sequence != r.sequence {
        return Err(bad());
    }
    let (active, _) = record::history_source(broker, &r.batch_id, r.sequence, r.rollback)?;
    let op = active
        .items
        .iter()
        .find(|op| &op.path == path)
        .ok_or_else(bad)?;
    let (before, after) = if r.rollback {
        (Some(op.target_revision), op.previous_revision)
    } else {
        (op.previous_revision, Some(op.target_revision))
    };
    let (revision, current) = if item.entry.id == id(&active, path, "new") {
        (after.ok_or_else(bad)?, true)
    } else if item.entry.id == id(&active, path, "previous") {
        (before.ok_or_else(bad)?, false)
    } else {
        return Err(bad());
    };
    if item.entry != entry(broker, &active, op, revision, current)? {
        return Err(bad());
    }
    Ok(())
}
pub(super) fn visible(broker: &FsVaultBroker, item: &Item) -> Result<bool, BrokerError> {
    item.batch
        .as_ref()
        .map(|r| {
            record::history_source(broker, &r.batch_id, r.sequence, r.rollback)
                .map(|(_, visible)| visible)
        })
        .unwrap_or(Ok(true))
}

pub(crate) fn preflight(
    broker: &FsVaultBroker,
    path: &VaultPath,
    reserve: usize,
) -> Result<(), BrokerError> {
    catalog::preflight(broker, path)?;
    let catalog = catalog::load(broker, path)?;
    if catalog.entries.len() > super::store::MAX_ENTRIES - reserve
        || serde_json::to_vec(&catalog).map_err(|_| bad())?.len()
            > super::store::CATALOG_BYTES - reserve * 4096
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(())
}
