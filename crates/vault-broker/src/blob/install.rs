//! 无额外大读窗口的 sealed 安装：最终元数据检查后 rename，再全量分块核对结果。
mod history;
pub(crate) use history::HistoryPins;
#[cfg(test)]
mod faults;
use super::{
    current::{self, Current},
    file, store,
};
use crate::mutation::record::Operation;
use crate::{
    BrokerError, FsVaultBroker, Precondition,
    writer::{Identity, directory},
};
#[cfg(test)]
pub(crate) use faults::before_history_check_test;
use std::path::Path;

pub(crate) fn sealed(
    broker: &FsVaultBroker,
    op: &Operation,
    old: &Current,
    stage: &file::ContentProof,
    history: &HistoryPins,
) -> Result<(), BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    current::check(broker, &op.path, old)?;
    let stages = directory(&broker.root, ".workbench/staging")?;
    file::check(&stages, &store::logical(&op.stage.name)?, &stage.node)?;
    history.check(broker, op)?;
    let (parent, name) = op
        .path
        .as_str()
        .rsplit_once('/')
        .unwrap_or((".", op.path.as_str()));
    let target = directory(&broker.root, parent)?;
    let identity = Identity::directory(&target)?;
    #[cfg(test)]
    before_history_check_test(broker, op, true)?;
    // 打开最终目标目录以后再检查；Installing 已持久，失败只能保留未知。
    history.check(broker, op)?;
    match op.precondition {
        Precondition::MustNotExist => {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            rustix::fs::renameat_with(
                &stages,
                &op.stage.name,
                &target,
                name,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(|e| BrokerError::io("install stream create", e.into()))?;
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            return Err(BrokerError::WriterLimitExceeded);
        }
        Precondition::Match(_) => stages
            .rename(&op.stage.name, &target, name)
            .map_err(|e| BrokerError::io("install stream replacement", e))?,
    }
    FsVaultBroker::sync_directory(&stages, Path::new("."))?;
    FsVaultBroker::sync_directory(&target, Path::new("."))?;
    #[cfg(test)]
    crate::mutation::install::after_rename_test(broker, &target, std::ffi::OsStr::new(name))?;
    let installed = store::verify(
        broker,
        &broker.root,
        op.path.as_str(),
        &op.path,
        op.target_revision,
    )?;
    if installed.identity != stage.identity
        || installed.node.file.identity != op.stage.identity
        || Identity::directory(&directory(&broker.root, parent)?)? != identity
    {
        return Err(BrokerError::CaptureChanged);
    }
    broker.writer.check(&broker.root, &broker.data)
}
