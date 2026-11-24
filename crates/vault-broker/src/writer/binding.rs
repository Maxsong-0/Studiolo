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
        Self::observe(root, data, vault_uuid, VaultUuid::new(), Ok)
    }

    /// 按两个根里现在的样子拼一份绑定。`original` 把看到的身份换成绑定该记的：
    /// 写者原样；封存副本换回定格那一刻的原身份，没见过的就报错。
    fn observe(
        root: &Dir,
        data: &Dir,
        vault_uuid: VaultUuid,
        pair_id: VaultUuid,
        original: impl Fn(Identity) -> Result<Identity, BrokerError>,
    ) -> Result<Self, BrokerError> {
        let directory = |dir: &Dir| original(Identity::directory(dir)?);
        let control = lease::child(root, ".workbench")?;
        let (bytes, marker) = disk::read_identified(&control, "vault.json", disk::IDENTITY_LIMIT)?
            .ok_or(BrokerError::CorruptIdentity)?;
        if crate::VaultIdentity::decode(&bytes)? != vault_uuid {
            return Err(BrokerError::WriterBindingMismatch);
        }
        Ok(Self {
            format_version: 1,
            pair_id,
            vault_uuid,
            canonical: directory(root)?,
            data: directory(data)?,
            control: directory(&control)?,
            marker: original(marker)?,
            staging: directory(&lease::child(&control, "staging")?)?,
            history: directory(&lease::child(data, "history")?)?,
            idempotency: directory(&lease::child(data, "idempotency")?)?,
            snapshots: directory(&lease::child(data, "snapshots")?)?,
        })
    }

    pub(crate) fn verify(
        &self,
        root: &Dir,
        data: &Dir,
        uuid: VaultUuid,
    ) -> Result<(), BrokerError> {
        self.verify_as(root, data, uuid, Ok)
    }

    /// 同 [`Binding::verify`]，身份先经 `original` 换过再比（见 [`Binding::observe`]）。
    pub(crate) fn verify_as(
        &self,
        root: &Dir,
        data: &Dir,
        uuid: VaultUuid,
        original: impl Fn(Identity) -> Result<Identity, BrokerError>,
    ) -> Result<(), BrokerError> {
        if self.format_version != 1 {
            return Err(BrokerError::CorruptWriterState);
        }
        if *self != Self::observe(root, data, uuid, self.pair_id, original)? {
            return Err(BrokerError::WriterBindingMismatch);
        }
        Ok(())
    }
}
