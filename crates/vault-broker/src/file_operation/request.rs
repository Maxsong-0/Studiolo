//! 首个 Preparing 固定完整意图；副本缺失只能由同一主记录重建，不能采纳孤儿授权。
use super::{
    disk::{self, Folder},
    frozen::Frozen,
    model::FilePresence,
    record::Active,
};
use crate::{BrokerError, FsVaultBroker, writer::disk as codec};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub version: u32,
    pub frozen: Frozen,
    pub sequence: u64,
    pub reserved_bytes: u64,
    pub observed_before: FilePresence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<FilePresence>,
}
impl Request {
    pub(crate) fn from_active(active: &Active) -> Result<Self, BrokerError> {
        Ok(Self {
            version: active.frozen.version,
            frozen: active.frozen.clone(),
            sequence: active.sequence,
            reserved_bytes: active.reserved_bytes,
            observed_before: active.before.presence()?,
            secondary: active
                .rename
                .as_ref()
                .map(|r| r.before.presence())
                .transpose()?,
        })
    }
    pub(crate) fn validate(&self, broker: &FsVaultBroker) -> Result<(), BrokerError> {
        self.frozen.validate(broker.writer.binding.pair_id)?;
        if self.frozen.version == 2 && !crate::mutation::load(broker)?.rename_enabled {
            return Err(BrokerError::CorruptWriterState);
        }
        super::record::validate_presence(&self.observed_before)?;
        if let Some(p) = &self.secondary {
            super::record::validate_presence(p)?;
        }
        if self.version != self.frozen.version
            || (self.version == 2) != self.secondary.is_some()
            || self.sequence == 0
            || self.reserved_bytes == 0
            || self.reserved_bytes > self.frozen.limits.retained_bytes_limit
            || self.frozen.vault_uuid != broker.vault_uuid
            || self.frozen.limits != super::store::required(broker)?.limits
        {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(())
    }
}
pub(crate) fn load(broker: &FsVaultBroker, id: &str) -> Result<Option<Request>, BrokerError> {
    let Some(bytes) = disk::read(broker, Folder::Records, &format!("{id}.json"))? else {
        return Ok(None);
    };
    let value: Request = codec::decode(&bytes)?;
    value.validate(broker)?;
    if value.frozen.operation_id != id {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(Some(value))
}
pub(crate) fn ensure(broker: &FsVaultBroker, active: &Active) -> Result<Request, BrokerError> {
    let value = Request::from_active(active)?;
    value.validate(broker)?;
    disk::save(
        broker,
        Folder::Records,
        &format!("{}.json", value.frozen.operation_id),
        &value,
        None,
    )?;
    Ok(value)
}
