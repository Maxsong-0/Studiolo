//! 两端保存相同的不可变配对；复制 UUID 不能替代目录和 marker 身份。

use super::{Identity, disk, lease};
use crate::{BrokerError, VaultUuid};
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    pub(crate) format_version: u32,
    pub(crate) pair_id: VaultUuid,
    pub(crate) vault_uuid: VaultUuid,
    pub(crate) canonical: Identity,
    pub(crate) data: Identity,
    pub(crate) control: Identity,
    pub(crate) marker: Identity,
    pub(crate) staging: Identity,
    pub(crate) history: Identity,
    pub(crate) idempotency: Identity,
    pub(crate) snapshots: Identity,
}

impl Binding {
    pub(crate) fn capture(
        root: &Dir,
        data: &Dir,
        vault_uuid: VaultUuid,
    ) -> Result<Self, BrokerError> {
        let control = lease::child(root, ".workbench")?;
        let (bytes, marker) = disk::read_identified(&control, "vault.json", disk::IDENTITY_LIMIT)?
            .ok_or(BrokerError::CorruptIdentity)?;
        if crate::VaultIdentity::decode(&bytes)? != vault_uuid {
            return Err(BrokerError::WriterBindingMismatch);
        }
        Ok(Self {
            format_version: 1,
            pair_id: VaultUuid::new(),
            vault_uuid,
            canonical: Identity::directory(root)?,
            data: Identity::directory(data)?,
            control: Identity::directory(&control)?,
            marker,
            staging: Identity::directory(&lease::child(&control, "staging")?)?,
            history: Identity::directory(&lease::child(data, "history")?)?,
            idempotency: Identity::directory(&lease::child(data, "idempotency")?)?,
            snapshots: Identity::directory(&lease::child(data, "snapshots")?)?,
        })
    }

    pub(crate) fn verify(
        &self,
        root: &Dir,
        data: &Dir,
        uuid: VaultUuid,
    ) -> Result<(), BrokerError> {
        if self.format_version != 1 {
            return Err(BrokerError::CorruptWriterState);
        }
        let mut actual = Self::capture(root, data, uuid)?;
        actual.pair_id = self.pair_id;
        if *self != actual {
            return Err(BrokerError::WriterBindingMismatch);
        }
        Ok(())
    }
}
