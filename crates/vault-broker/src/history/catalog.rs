//! 每路径有界索引；完成阶段可重做索引，未推进水位的条目不对读者发布。

use super::{HistoryEntry, legacy, store};
use crate::{BrokerError, FsVaultBroker, Revision, VaultPath, VaultUuid, sha256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Item {
    pub sequence: u64,
    pub entry: HistoryEntry,
    pub recording: Option<crate::recorded::store::RecordingRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_operation_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    pub version: u32,
    pub pair_id: VaultUuid,
    pub path: VaultPath,
    pub entries: Vec<Item>,
    /// 已从查询目录退役、等待可靠删除的对象；崩溃后可以继续清理。
    pub garbage: Vec<Revision>,
    #[serde(skip)]
    pub original: Option<String>,
}

pub(super) fn name(path: &VaultPath) -> String {
    format!("catalogs/{}.json", sha256(path.as_str().as_bytes()))
}
pub(super) fn load(broker: &FsVaultBroker, path: &VaultPath) -> Result<Catalog, BrokerError> {
    if path.is_protected() {
        return Err(BrokerError::ProtectedPath(path.clone()));
    }
    let value = match store::read(broker, &name(path), store::CATALOG_BYTES)? {
        Some(bytes) => {
            let mut value: Catalog = crate::writer::disk::decode(&bytes)?;
            value.original = Some(sha256(&bytes));
            value
        }
        None => Catalog {
            version: 1,
            pair_id: broker.writer.binding.pair_id,
            path: path.clone(),
            entries: legacy::load(broker, path)?,
            garbage: Vec::new(),
            original: None,
        },
    };
    validate(broker, path, &value)?;
    Ok(value)
}
pub(super) fn validate(
    broker: &FsVaultBroker,
    path: &VaultPath,
    value: &Catalog,
) -> Result<(), BrokerError> {
    if !matches!(value.version, 1..=3)
        || value.pair_id != broker.writer.binding.pair_id
        || &value.path != path
        || path.is_protected()
    {
        return Err(BrokerError::CorruptWriterState);
    }
    if value.entries.len() > store::MAX_ENTRIES || value.garbage.len() > store::MAX_ENTRIES {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let mut ids = BTreeSet::new();
    for item in &value.entries {
        super::evidence::validate(broker, path, item)?;
        let e = &item.entry;
        if !crate::writer::disk::hex(&e.id)
            || !ids.insert(&e.id)
            || e.bytes
                > (if value.version >= 2 {
                    crate::blob::OBJECT_LIMIT
                } else {
                    crate::writer::disk::CONTENT_LIMIT as u64
                })
            || e.authored_at_ms.is_some_and(|v| v < 0)
            || e.archived_at_ms.is_some_and(|v| v < 0)
        {
            return Err(BrokerError::CorruptWriterState);
        }
        for actor in [&e.author, &e.archived_by].into_iter().flatten() {
            crate::MutationMetadata::new(actor.clone(), crate::MutationKind::Replace)
                .map_err(|_| BrokerError::CorruptWriterState)?;
        }
    }
    let mut garbage = BTreeSet::new();
    for revision in &value.garbage {
        if !garbage.insert(revision.to_string())
            || value.entries.iter().any(|i| &i.entry.revision == revision)
        {
            return Err(BrokerError::CorruptWriterState);
        }
    }
    Ok(())
}
pub(super) fn save(broker: &FsVaultBroker, value: &mut Catalog) -> Result<(), BrokerError> {
    let state = crate::mutation::load(broker)?;
    if state.format_version >= 5 {
        value.version = value.version.max(3);
    } else if state.format_version >= 4 {
        value.version = value.version.max(2);
    }
    validate(broker, &value.path, value)?;
    store::save_catalog(broker, &name(&value.path), value, value.original.as_deref())?;
    value.original = Some(super::query::checksum(value)?);
    Ok(())
}

pub(crate) fn preflight(broker: &FsVaultBroker, path: &VaultPath) -> Result<(), BrokerError> {
    let catalog = load(broker, path)?;
    if catalog.entries.len() > store::MAX_ENTRIES - 2 || !catalog.garbage.is_empty() {
        return Err(BrokerError::WriterLimitExceeded);
    }
    // 两个最大主体条目仍须在安装前有确定空间，不能安装后才发现索引超限。
    let bytes = serde_json::to_vec(&catalog).map_err(|_| BrokerError::CorruptWriterState)?;
    if bytes.len() > store::CATALOG_BYTES - 8192 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(())
}
