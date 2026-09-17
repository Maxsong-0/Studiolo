//! 大历史只输出精确身份；正文复制由 Broker 内部使用固定 capability 与 Revision 完成。
use crate::blob::{file::ContentProof, store};
use crate::writer::directory;
use crate::{BrokerError, FsVaultBroker, Revision, VaultPath};

pub(super) fn object(
    broker: &FsVaultBroker,
    path: &VaultPath,
    revision: Revision,
) -> Result<ContentProof, BrokerError> {
    store::verify(
        broker,
        &directory(&broker.data, "history")?,
        &revision.to_string(),
        path,
        revision,
    )
}
pub(crate) fn selected(
    broker: &FsVaultBroker,
    path: &VaultPath,
    revision: Revision,
) -> Result<crate::BlobIdentity, BrokerError> {
    let state = super::query::readable(broker)?;
    let catalog = super::catalog::load(broker, path)?;
    let checksum = super::query::checksum(&catalog)?;
    let entries: Vec<_> = catalog
        .entries
        .iter()
        .filter(|item| item.sequence <= state.committed_through && item.entry.revision == revision)
        .collect();
    if entries.is_empty() {
        return Err(BrokerError::NotFound(path.clone()));
    }
    let proof = object(broker, path, revision)?;
    if entries
        .iter()
        .any(|item| item.entry.bytes != proof.identity.bytes)
    {
        return Err(BrokerError::CorruptWriterState);
    }
    super::query::unchanged(broker, &state)?;
    if super::query::checksum(&super::catalog::load(broker, path)?)? != checksum {
        return Err(BrokerError::HistoryChanged);
    }
    Ok(proof.identity)
}
