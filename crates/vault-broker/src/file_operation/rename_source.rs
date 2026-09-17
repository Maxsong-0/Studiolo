//! 第二子步只摘除原源对象；目录自身 Stamp 变化不能放宽源文件或目标的证明。
use super::{observation::Observation, record::Active, *};
use crate::{
    BrokerError, FsVaultBroker,
    blob::{file, store as blobs},
    writer::directory,
};
use std::path::Path;
pub(crate) fn prepare(broker: &FsVaultBroker, active: &mut Active) -> Result<(), BrokerError> {
    let (to, r) = rename_model::get(active)?;
    let target = r
        .executed
        .as_ref()
        .and_then(Observation::proof)
        .ok_or(BrokerError::CorruptWriterState)?;
    let original = active
        .before
        .proof()
        .ok_or(BrokerError::CorruptWriterState)?;
    let source = rename_target::observe_unchanged(broker, &active.frozen.path, original)?;
    // 目标校验可能大读；之后再次复验源新的完整节点证明。
    rename_target::observe_unchanged(broker, to, target)?;
    source.check(broker, &active.frozen.path)?;
    let roots = super::store::required(broker)?;
    if file::missing(
        &roots.quarantine,
        &blobs::logical(&format!("{}.object", active.frozen.operation_id))?,
    )?
    .is_none()
    {
        return Err(BrokerError::CaptureChanged);
    }
    active
        .rename
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?
        .source_check = Some(source);
    Ok(())
}
pub(crate) fn detach(broker: &FsVaultBroker, active: &mut Active) -> Result<(), BrokerError> {
    let (to, r) = rename_model::get(active)?;
    let to = to.clone();
    let source = r
        .source_check
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    source.check(broker, &active.frozen.path)?;
    let target = r
        .executed
        .as_ref()
        .and_then(Observation::proof)
        .ok_or(BrokerError::CorruptWriterState)?
        .clone();
    file::check(&broker.root, &to, &target.node)?;
    let path = &active.frozen.path;
    let (prefix, name) = path
        .as_str()
        .rsplit_once('/')
        .unwrap_or((".", path.as_str()));
    let parent = directory(&broker.root, prefix)?;
    let parents = file::parents(&broker.root, path)?;
    let roots = super::store::required(broker)?;
    let object = format!("{}.object", active.frozen.operation_id);
    let old = active
        .before
        .proof()
        .ok_or(BrokerError::CorruptWriterState)?;
    let revision = old.revision.ok_or(BrokerError::CorruptWriterState)?;
    crate::mutation::fault(broker, 77)?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    rustix::fs::renameat_with(
        &parent,
        name,
        &roots.quarantine,
        &object,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|e| BrokerError::io("保全摘除重命名源", e.into()))?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(BrokerError::WriterLimitExceeded);
    crate::mutation::fault(broker, 88)?;
    FsVaultBroker::sync_directory(&parent, Path::new("."))?;
    crate::mutation::fault(broker, 89)?;
    FsVaultBroker::sync_directory(&roots.quarantine, Path::new("."))?;
    crate::mutation::fault(broker, 78)?;
    let moved = blobs::verify(broker, &roots.quarantine, &object, path, revision)?;
    if moved.identity != old.identity || moved.node.file.identity != old.node.file.identity {
        return Err(BrokerError::CaptureChanged);
    }
    let missing = Observation::observe(broker, path)?;
    let Observation::Missing { parents: after } = &missing else {
        return Err(BrokerError::CaptureChanged);
    };
    rename_target::same_parents(&parents, after)?;
    // 摘源会改变共同祖先；重读目标但不改写第一子步的历史 NodeProof。
    rename_target::observe_unchanged(broker, &to, &target)?;
    missing.check(broker, path)?;
    super::store::required(broker)?;
    active.executed = Some(missing);
    active.quarantine = Some(moved);
    active.effect_at_ms = crate::history::store::now();
    active.phase = FileOperationPhase::SourceDetached;
    Ok(())
}
