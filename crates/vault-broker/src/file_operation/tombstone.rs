//! 墓碑只引用已确认的删除效果与旧字节；本片不提供任何自动或显式 GC。
use super::{
    FileOperationOutcome, FilePresence,
    disk::{self, Folder},
    record::{Active, Terminal},
};
use crate::{
    BrokerError, FsVaultBroker, MutationActor, Revision, VaultPath, blob::file::ContentProof,
    writer::disk as codec,
};
use serde::{Deserialize, Serialize};

/// 最近删除条目；时间未知时保持 None，不用文件时间猜测。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TombstoneEntry {
    /// 不透明墓碑选择器。
    pub id: String,
    /// 唯一允许恢复的原路径。
    pub path: VaultPath,
    /// 原字节与原路径绑定的版本。
    pub revision: Revision,
    /// 原始字节摘要。
    pub sha256: String,
    /// 精确逻辑字节数。
    pub bytes: u64,
    /// 已记录执行效果的服务器时间，不是 Obsidian 客户端时间。
    pub deleted_at_ms: Option<i64>,
    /// 原请求的实际主体。
    pub deleted_by: MutationActor,
    /// 至少保留到此时间；本片 GC 关闭，超过此时间也不自动删除。
    pub retain_until_ms: Option<i64>,
}
/// 固定目录代次的有界页面；墓碑只增不删，改变后从第一页重新查询。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TombstonePage {
    /// 可恢复的已确认墓碑。
    pub entries: Vec<TombstoneEntry>,
    /// 不透明下一页游标。
    pub next_cursor: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tombstone {
    pub version: u32,
    pub terminal: Terminal,
    pub history: ContentProof,
    pub quarantine: ContentProof,
}
impl Tombstone {
    pub(crate) fn entry(&self) -> Result<TombstoneEntry, BrokerError> {
        let FilePresence::Present {
            revision,
            sha256,
            bytes,
        } = &self.terminal.observed_before
        else {
            return Err(BrokerError::CorruptWriterState);
        };
        let frozen = &self.terminal.frozen;
        Ok(TombstoneEntry {
            id: frozen.operation_id.clone(),
            path: frozen.path.clone(),
            revision: *revision,
            sha256: sha256.clone(),
            bytes: *bytes,
            deleted_at_ms: self.terminal.effect_at_ms,
            deleted_by: frozen.actor.clone(),
            retain_until_ms: self
                .terminal
                .effect_at_ms
                .and_then(|v| v.checked_add(super::limits::MINIMUM_RETENTION_MS)),
        })
    }
    fn validate(&self, broker: &FsVaultBroker) -> Result<(), BrokerError> {
        self.terminal.validate(broker.writer.binding.pair_id)?;
        let entry = self.entry()?;
        if self.version != self.terminal.frozen.version
            || self.terminal.receipt.outcome != FileOperationOutcome::Committed
            || self.terminal.receipt.tombstone_id.as_ref() != Some(&entry.id)
            || self.history.revision != Some(entry.revision)
            || self.quarantine.revision != Some(entry.revision)
            || self.history.identity != self.quarantine.identity
            || self.history.identity.bytes != entry.bytes
            || self.history.identity.sha256 != entry.sha256
        {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(())
    }
}
pub(crate) fn save(broker: &FsVaultBroker, active: &Active) -> Result<(), BrokerError> {
    let value = Tombstone {
        version: active.frozen.version,
        terminal: active
            .terminal
            .clone()
            .ok_or(BrokerError::CorruptWriterState)?,
        history: active
            .history
            .clone()
            .ok_or(BrokerError::CorruptWriterState)?,
        quarantine: active
            .quarantine
            .clone()
            .ok_or(BrokerError::CorruptWriterState)?,
    };
    value.validate(broker)?;
    disk::save(
        broker,
        Folder::Tombstones,
        &format!("{}.json", active.frozen.operation_id),
        &value,
        None,
    )
}
pub(crate) fn load(broker: &FsVaultBroker, id: &str) -> Result<Tombstone, BrokerError> {
    if !codec::hex(id) {
        return Err(BrokerError::IdempotencyConflict);
    }
    let bytes = disk::read(broker, Folder::Tombstones, &format!("{id}.json"))?
        .ok_or_else(|| BrokerError::InvalidArtifact("墓碑不存在".into()))?;
    let value: Tombstone = codec::decode(&bytes)?;
    value.validate(broker)?;
    if value.terminal.frozen.operation_id != id
        || super::terminal::load(broker, &value.terminal.frozen)?.as_ref() != Some(&value.terminal)
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(value)
}
pub(crate) fn source(
    broker: &FsVaultBroker,
    id: &str,
    path: &VaultPath,
) -> Result<Tombstone, BrokerError> {
    let value = load(broker, id)?;
    let request = super::request::load(broker, id)?.ok_or(BrokerError::CorruptWriterState)?;
    if &value.terminal.frozen.path != path {
        return Err(BrokerError::IdempotencyConflict);
    }
    if !super::audit::confirmed(broker, &super::audit::outcome(&request, &value.terminal)?)? {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(value)
}
pub(crate) fn protected(
    broker: &FsVaultBroker,
    path: &VaultPath,
) -> Result<Vec<Revision>, BrokerError> {
    if super::store::open(broker)?.is_none() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for name in disk::names(broker, Folder::Records)? {
        let value = super::request::load(broker, name.trim_end_matches(".json"))?
            .ok_or(BrokerError::CorruptWriterState)?;
        if let super::FileOperationKind::Rename { to, .. } = &value.frozen.kind
            && to == path
        {
            if let Some(terminal) = super::terminal::load(broker, &value.frozen)? {
                if let Some(revision) = terminal.rename.and_then(|r| r.target_revision) {
                    result.push(revision);
                }
            } else {
                let state = crate::mutation::load(broker)?;
                let active = state
                    .file_operation
                    .as_ref()
                    .filter(|a| a.frozen == value.frozen)
                    .ok_or(BrokerError::CorruptWriterState)?;
                if let Some(revision) = active.rename.as_ref().and_then(|r| r.target_revision) {
                    result.push(revision);
                }
            }
        }
        if &value.frozen.path == path {
            if let FilePresence::Present { revision, .. } = value.observed_before {
                result.push(revision);
            }
            if let super::FileOperationKind::RestoreTombstone { tombstone_id } = &value.frozen.kind
            {
                result.push(load(broker, tombstone_id)?.entry()?.revision);
            }
        }
    }
    result.sort();
    result.dedup();
    Ok(result)
}
