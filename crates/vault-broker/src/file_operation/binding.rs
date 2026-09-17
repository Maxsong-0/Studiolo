//! 新能力单独固定目录身份和审批上限；只在显式 enable 发布完整 schema。
use super::limits::FileOperationLimits;
use crate::{
    BrokerError, FsVaultBroker, VaultUuid,
    writer::{Identity, directory, disk},
};
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

pub(crate) const NAME: &str = "file-operations-v1";
pub(crate) const PREPARING: &str = "file-operations-v1.preparing";
pub(crate) const QUARANTINE: &str = ".workbench/deleted-objects";
pub(crate) const QUARANTINE_PREPARING: &str = ".workbench/deleted-objects.preparing";
pub(crate) const CHILDREN: [&str; 4] = ["records", "tombstones", "acks", "evidence"];

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    pub version: u32,
    pub pair_id: VaultUuid,
    pub vault_uuid: VaultUuid,
    pub root: Identity,
    pub children: Vec<(String, Identity)>,
    pub quarantine: Identity,
    pub limits: FileOperationLimits,
}

pub(crate) struct Pin {
    pub binding: Binding,
    pub file: disk::FilePin,
}

impl Binding {
    pub(crate) fn capture(
        broker: &FsVaultBroker,
        root: &Dir,
        quarantine: &Dir,
        limits: FileOperationLimits,
    ) -> Result<Self, BrokerError> {
        limits.validate()?;
        let mut children = Vec::new();
        for name in CHILDREN {
            children.push((name.into(), Identity::directory(&directory(root, name)?)?));
        }
        let value = Self {
            version: 1,
            pair_id: broker.writer.binding.pair_id,
            vault_uuid: broker.vault_uuid,
            root: Identity::directory(root)?,
            children,
            quarantine: Identity::directory(quarantine)?,
            limits,
        };
        if value.quarantine.dev != Identity::directory(&broker.root)?.dev {
            return Err(BrokerError::WriterBindingMismatch);
        }
        Ok(value)
    }
    pub(crate) fn validate(
        &self,
        broker: &FsVaultBroker,
        root: &Dir,
        quarantine: &Dir,
    ) -> Result<(), BrokerError> {
        self.limits.validate()?;
        if self.version != 1
            || self.pair_id != broker.writer.binding.pair_id
            || self.vault_uuid != broker.vault_uuid
            || self.root != Identity::directory(root)?
            || self.quarantine != Identity::directory(quarantine)?
            || self.quarantine.dev != Identity::directory(&broker.root)?.dev
            || self.children.len() != CHILDREN.len()
        {
            return Err(BrokerError::WriterBindingMismatch);
        }
        for ((name, identity), expected) in self.children.iter().zip(CHILDREN) {
            if name != expected || *identity != Identity::directory(&directory(root, expected)?)? {
                return Err(BrokerError::WriterBindingMismatch);
            }
        }
        Ok(())
    }
}
