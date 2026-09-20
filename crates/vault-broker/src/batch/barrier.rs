//! 根内持久读屏障让独立 canonical 窄读也能拒绝半批；不含正文。
use crate::{BrokerError, FsVaultBroker, mutation, writer::disk};
use std::path::Path;
const NAME: &str = ".workbench/batch-active.json";

pub(crate) fn blocked(root: &cap_std::fs::Dir) -> Result<bool, BrokerError> {
    Ok(disk::read(root, NAME, disk::IDENTITY_LIMIT)?.is_some())
}
pub(super) fn create(broker: &FsVaultBroker, state: &mutation::State) -> Result<(), BrokerError> {
    let active = state
        .batch
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    disk::create(
        &broker.root,
        NAME,
        &(
            broker.writer.binding.pair_id,
            active.sequence,
            &active.fingerprint,
        ),
        || broker.writer.check(&broker.root, &broker.data),
    )
}
pub(crate) fn clear_if_idle(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    if mutation::load(broker)?.batch.is_some() {
        return Ok(());
    }
    if blocked(&broker.root)? {
        broker.writer.check(&broker.root, &broker.data)?;
        let control = crate::writer::directory(&broker.root, ".workbench")?;
        control
            .remove_file("batch-active.json")
            .map_err(|e| BrokerError::io("clear batch read barrier", e))?;
        FsVaultBroker::sync_directory(&control, Path::new("."))?;
        broker.writer.check(&broker.root, &broker.data)?;
    }
    Ok(())
}
