//! 第一子步只创建不存在目标；源始终保留，执行证据单独持久化。
use super::{observation::Observation, record::Active, *};
use crate::{
    BrokerError, FsVaultBroker,
    blob::{file, store as blobs},
    writer::directory,
};
use std::path::Path;
pub(crate) fn install(broker: &FsVaultBroker, active: &mut Active) -> Result<(), BrokerError> {
    rename_prepare::check(broker, active)?;
    let (to, r) = rename_model::get(active)?;
    let to = to.clone();
    let projected = r.history.as_ref().ok_or(BrokerError::CorruptWriterState)?;
    let revision = projected.revision.ok_or(BrokerError::CorruptWriterState)?;
    let parents = file::parents(&broker.root, &to)?;
    let (prefix, name) = to.as_str().rsplit_once('/').unwrap_or((".", to.as_str()));
    let target = directory(&broker.root, prefix)?;
    let stages = directory(&broker.root, ".workbench/staging")?;
    let stage = active
        .stage
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    crate::mutation::fault(broker, 73)?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    rustix::fs::renameat_with(
        &stages,
        &stage.name,
        &target,
        name,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|e| BrokerError::io("创建重命名目标", e.into()))?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(BrokerError::WriterLimitExceeded);
    crate::mutation::fault(broker, 86)?;
    FsVaultBroker::sync_directory(&stages, Path::new("."))?;
    crate::mutation::fault(broker, 87)?;
    FsVaultBroker::sync_directory(&target, Path::new("."))?;
    crate::mutation::fault(broker, 74)?;
    let installed = blobs::verify(broker, &broker.root, to.as_str(), &to, revision)?;
    if installed.identity != projected.identity || installed.node.file.identity != stage.identity {
        return Err(BrokerError::CaptureChanged);
    }
    same_parents(&parents, &installed.node.parents)?;
    super::store::required(broker)?;
    let r = active
        .rename
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?;
    r.executed = Some(Observation::Present { proof: installed });
    r.target_effect_at_ms = crate::history::store::now();
    active.phase = FileOperationPhase::TargetInstalled;
    Ok(())
}
pub(crate) fn same_parents(
    a: &[(String, file::Stamp)],
    b: &[(String, file::Stamp)],
) -> Result<(), BrokerError> {
    if a.len() != b.len()
        || a.iter()
            .zip(b)
            .any(|(a, b)| a.0 != b.0 || a.1.identity != b.1.identity)
    {
        return Err(BrokerError::CaptureChanged);
    }
    Ok(())
}
// 只在自身同目录变更之后使用；旧文件完整 Stamp 与所有父 inode 仍须一致。
pub(crate) fn observe_unchanged(
    broker: &FsVaultBroker,
    path: &crate::VaultPath,
    expected: &file::ContentProof,
) -> Result<Observation, BrokerError> {
    let observed = Observation::observe(broker, path)?;
    let actual = observed.proof().ok_or(BrokerError::CaptureChanged)?;
    if actual.identity != expected.identity
        || actual.revision != expected.revision
        || actual.node.file != expected.node.file
    {
        return Err(BrokerError::CaptureChanged);
    }
    same_parents(&actual.node.parents, &expected.node.parents)?;
    Ok(observed)
}
