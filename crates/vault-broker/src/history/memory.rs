//! 无IO替身使用同一变更模型；先准备完整历史，再交换正文与元数据。
mod query;
use super::{ChangeEntry, ChangeKind, HistoryEntry};
use crate::{BrokerError, MutationActor, Revision, VaultPath};
use std::collections::BTreeMap;
#[derive(Default)]
pub(crate) struct Journal {
    sequence: u64,
    entries: BTreeMap<VaultPath, Vec<HistoryEntry>>,
    changes: Vec<ChangeEntry>,
}
pub(crate) struct Prepared {
    sequence: u64,
    entries: Vec<(VaultPath, HistoryEntry)>,
    changes: Vec<ChangeEntry>,
}
pub(crate) struct Change {
    pub path: VaultPath,
    pub before: Option<(Revision, u64)>,
    pub after: Option<(Revision, u64)>,
    pub kind: ChangeKind,
}
impl Journal {
    pub(crate) fn prepare(
        &self,
        changes: Vec<Change>,
        actor: Option<MutationActor>,
    ) -> Result<Prepared, BrokerError> {
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        let at = super::store::now();
        let mut result = Prepared {
            sequence,
            entries: Vec::new(),
            changes: Vec::new(),
        };
        for change in changes {
            let entries = self.entries.get(&change.path);
            if entries.is_some_and(|v| v.len() > super::store::MAX_ENTRIES - 2) {
                return Err(BrokerError::WriterLimitExceeded);
            }
            let id = |role| {
                crate::sha256(
                    format!("memory-history-v1:{sequence}:{}:{role}", change.path).as_bytes(),
                )
            };
            for ((revision, bytes), current) in change
                .before
                .into_iter()
                .map(|r| (r, false))
                .chain(change.after.map(|r| (r, true)))
            {
                if !current
                    && change.after.is_some()
                    && entries.is_some_and(|v| v.iter().any(|i| i.revision == revision))
                {
                    continue;
                }
                result.entries.push((
                    change.path.clone(),
                    HistoryEntry {
                        id: id(if current { "new" } else { "previous" }),
                        revision,
                        bytes,
                        authored_at_ms: if current { at } else { None },
                        author: if current { actor.clone() } else { None },
                        archived_at_ms: at,
                        archived_by: actor.clone(),
                    },
                ));
            }
            result.changes.push(ChangeEntry {
                id: id(if change.after.is_some() {
                    "new"
                } else {
                    "previous"
                }),
                sequence,
                path: change.path,
                kind: change.kind,
                actor: actor.clone(),
                previous_revision: change.before.map(|v| v.0),
                revision: change.after.map(|v| v.0),
                bytes: change.after.map_or(0, |v| v.1),
                at_ms: at,
                counterpart: None,
            });
        }
        result.changes.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(result)
    }
    pub(crate) fn commit(&mut self, prepared: Prepared) {
        self.sequence = prepared.sequence;
        for (path, entry) in prepared.entries {
            self.entries.entry(path).or_default().insert(0, entry);
        }
        self.changes.extend(prepared.changes);
    }
}
