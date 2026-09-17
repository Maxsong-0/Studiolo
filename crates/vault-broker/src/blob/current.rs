//! CAS 观察固定每级父目录与文件身份；缺失也是明确证据，不能由异常代替。
use super::{OBJECT_LIMIT, file};
use crate::{BrokerError, FsVaultBroker, Revision, VaultPath};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Current {
    Missing(Vec<(String, file::Stamp)>),
    Present(file::ContentProof),
}
impl Current {
    pub(crate) fn revision(&self) -> Option<Revision> {
        match self {
            Self::Missing(_) => None,
            Self::Present(proof) => proof.revision,
        }
    }
}
pub(crate) fn observe(broker: &FsVaultBroker, path: &VaultPath) -> Result<Current, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    let value = match file::missing(&broker.root, path)? {
        Some(parents) => {
            if file::missing(&broker.root, path)?.as_ref() != Some(&parents) {
                return Err(BrokerError::CaptureChanged);
            }
            Current::Missing(parents)
        }
        None => Current::Present(file::scan(
            &broker.root,
            path,
            OBJECT_LIMIT,
            Some((broker.vault_uuid, path)),
        )?),
    };
    broker.writer.check(&broker.root, &broker.data)?;
    Ok(value)
}
pub(crate) fn check(
    broker: &FsVaultBroker,
    path: &VaultPath,
    expected: &Current,
) -> Result<(), BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    match expected {
        Current::Present(proof) => file::check(&broker.root, path, &proof.node)?,
        Current::Missing(parents) => {
            if file::missing(&broker.root, path)?.as_ref() != Some(parents) {
                return Err(BrokerError::CaptureChanged);
            }
        }
    }
    broker.writer.check(&broker.root, &broker.data)
}
