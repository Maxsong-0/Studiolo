//! 完整建目录后才启用主 State v5；失败残留不会由 open_existing 隐式补齐。
use super::{binding::*, limits::FileOperationLimits};
use crate::{
    BrokerError, FsVaultBroker,
    writer::{directory, disk},
};
use std::path::Path;

pub(super) fn initialize(
    broker: &FsVaultBroker,
    limits: FileOperationLimits,
) -> Result<(), BrokerError> {
    limits.validate()?;
    broker.writer.check(&broker.root, &broker.data)?;
    let control = directory(&broker.root, ".workbench")?;
    // 两个固定准备域均必须不存在。所有变更只对已固定的直接父 FD 执行。
    for (parent, name) in [
        (&broker.data, NAME),
        (&broker.data, PREPARING),
        (&control, "deleted-objects"),
        (&control, "deleted-objects.preparing"),
    ] {
        match parent.symlink_metadata(name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(BrokerError::io("inspect file operation schema", error)),
            Ok(_) => return Err(BrokerError::CorruptWriterState),
        }
    }
    broker.writer.check(&broker.root, &broker.data)?;
    broker
        .data
        .create_dir(PREPARING)
        .map_err(|e| BrokerError::io("prepare file operation schema", e))?;
    control
        .create_dir("deleted-objects.preparing")
        .map_err(|e| BrokerError::io("prepare file quarantine", e))?;
    let root = directory(&broker.data, PREPARING)?;
    let quarantine = directory(&control, "deleted-objects.preparing")?;
    for name in CHILDREN {
        root.create_dir(name)
            .map_err(|e| BrokerError::io("create file operation namespace", e))?;
    }
    let binding = Binding::capture(broker, &root, &quarantine, limits)?;
    disk::create(&root, "binding.json", &binding, || {
        broker.writer.check(&broker.root, &broker.data)
    })?;
    for name in CHILDREN {
        FsVaultBroker::sync_directory(&root, Path::new(name))?;
    }
    FsVaultBroker::sync_directory(&root, Path::new("."))?;
    FsVaultBroker::sync_directory(&quarantine, Path::new("."))?;
    broker.writer.check(&broker.root, &broker.data)?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        rustix::fs::renameat_with(
            &control,
            "deleted-objects.preparing",
            &control,
            "deleted-objects",
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|e| BrokerError::io("publish file quarantine", e.into()))?;
        FsVaultBroker::sync_directory(&control, Path::new("."))?;
        rustix::fs::renameat_with(
            &broker.data,
            PREPARING,
            &broker.data,
            NAME,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|e| BrokerError::io("publish file operation schema", e.into()))?;
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(BrokerError::WriterLimitExceeded);
    FsVaultBroker::sync_directory(&broker.data, Path::new("."))?;
    broker.writer.check(&broker.root, &broker.data)
}
