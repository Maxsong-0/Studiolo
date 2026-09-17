//! Issued 之后所有异常都未知；只有移动后完整原字节、身份、父目录与缺失证明才能记录效果。
use super::{observation::Observation, record::Active, *};
use crate::{
    BrokerError, FsVaultBroker,
    blob::{file, store as blobs},
    writer::{Identity, directory},
};
use std::path::Path;

pub(crate) fn execute(broker: &FsVaultBroker, active: &mut Active) -> Result<(), BrokerError> {
    let roots = store::required(broker)?;
    let path = &active.frozen.path;
    let (parent, name) = path
        .as_str()
        .rsplit_once('/')
        .unwrap_or((".", path.as_str()));
    let target = directory(&broker.root, parent)?;
    let target_identity = Identity::directory(&target)?;
    let original_parents = crate::blob::file::parents(&broker.root, path)?;
    prepared::check(broker, active)?;
    crate::mutation::fault(broker, 53)?;
    #[cfg(test)]
    if broker
        .writer
        .fault
        .compare_exchange(
            60,
            0,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_ok()
    {
        // 仅契约使用：模拟最终名字检查之后、系统调用之前由外部写者替换目标。
        target
            .remove_file(name)
            .map_err(|e| BrokerError::io("inject source name race", e))?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let mut foreign = target
            .open_with(name, &options)
            .map_err(|e| BrokerError::io("inject foreign source", e))?;
        std::io::Write::write_all(&mut foreign, b"foreign")
            .map_err(|e| BrokerError::io("inject foreign bytes", e))?;
        foreign
            .sync_all()
            .map_err(|e| BrokerError::io("sync injected source", e))?;
    }
    let preserved = active
        .history
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    let revision = preserved.revision.ok_or(BrokerError::CorruptWriterState)?;
    let quarantine_name = format!("{}.object", active.frozen.operation_id);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    match &active.frozen.kind {
        FileOperationKind::Rename { .. } => return Err(BrokerError::CorruptWriterState),
        FileOperationKind::Delete { .. } => {
            rustix::fs::renameat_with(
                &target,
                name,
                &roots.quarantine,
                &quarantine_name,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(|e| BrokerError::io("detach deleted file without replacement", e.into()))?;
            FsVaultBroker::sync_directory(&target, Path::new("."))?;
            FsVaultBroker::sync_directory(&roots.quarantine, Path::new("."))?;
            crate::mutation::fault(broker, 54)?;
            let detached =
                blobs::verify(broker, &roots.quarantine, &quarantine_name, path, revision)?;
            let before = active
                .before
                .proof()
                .ok_or(BrokerError::CorruptWriterState)?;
            if detached.identity != before.identity
                || detached.node.file.identity != before.node.file.identity
            {
                return Err(BrokerError::CaptureChanged);
            }
            let current = Observation::observe(broker, path)?;
            if !matches!(current, Observation::Missing { .. }) {
                return Err(BrokerError::CaptureChanged);
            }
            active.executed = Some(current);
            active.quarantine = Some(detached);
            active.phase = FileOperationPhase::Detached;
        }
        FileOperationKind::RestoreTombstone { .. } => {
            let stages = directory(&broker.root, ".workbench/staging")?;
            let stage = active
                .stage
                .as_ref()
                .ok_or(BrokerError::CorruptWriterState)?;
            rustix::fs::renameat_with(
                &stages,
                &stage.name,
                &target,
                name,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(|e| BrokerError::io("restore deleted file without replacement", e.into()))?;
            FsVaultBroker::sync_directory(&stages, Path::new("."))?;
            FsVaultBroker::sync_directory(&target, Path::new("."))?;
            crate::mutation::fault(broker, 54)?;
            let installed = blobs::verify(broker, &broker.root, path.as_str(), path, revision)?;
            if installed.identity != preserved.identity
                || installed.node.file.identity != stage.identity
            {
                return Err(BrokerError::CaptureChanged);
            }
            active.executed = Some(Observation::Present { proof: installed });
            active.phase = FileOperationPhase::Installed;
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(BrokerError::WriterLimitExceeded);
    let after = file::parents(&broker.root, path)?;
    if original_parents.len() != after.len()
        || original_parents
            .iter()
            .zip(&after)
            .any(|(a, b)| a.0 != b.0 || a.1.identity != b.1.identity)
        || Identity::directory(&directory(&broker.root, parent)?)? != target_identity
    {
        return Err(BrokerError::CaptureChanged);
    }
    active.effect_at_ms = crate::history::store::now();
    store::required(broker)?;
    Ok(())
}
