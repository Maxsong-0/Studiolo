//! 固定 schema 与对象根的 capability 访问；不接受外部磁盘路径或调用方回调。
use super::{binding::*, limits::FileOperationLimits};
use crate::{
    BrokerError, FsVaultBroker,
    writer::{directory, disk},
};
use cap_std::fs::Dir;

pub(crate) struct Roots {
    pub records: Dir,
    pub quarantine: Dir,
    pub limits: FileOperationLimits,
}

fn exists(root: &Dir, name: &str) -> Result<bool, BrokerError> {
    match root.symlink_metadata(name) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(BrokerError::io("inspect file operation root", error)),
    }
}

pub(crate) fn open(broker: &FsVaultBroker) -> Result<Option<Roots>, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    if exists(&broker.data, PREPARING)? || exists(&broker.root, QUARANTINE_PREPARING)? {
        return Err(BrokerError::CorruptWriterState);
    }
    let present = exists(&broker.data, NAME)?;
    let quarantined = exists(&broker.root, QUARANTINE)?;
    let mut pin = broker
        .file_operation_pin
        .lock()
        .map_err(|_| BrokerError::CorruptWriterState)?;
    if !present && !quarantined {
        return if pin.is_some() {
            Err(BrokerError::WriterBindingMismatch)
        } else {
            Ok(None)
        };
    }
    if present != quarantined {
        return Err(BrokerError::CorruptWriterState);
    }
    let records = directory(&broker.data, NAME)?;
    let quarantine = directory(&broker.root, QUARANTINE)?;
    if pin.is_none() {
        let (bytes, identity) =
            disk::read_identified(&records, "binding.json", disk::IDENTITY_LIMIT)?
                .ok_or(BrokerError::CorruptWriterState)?;
        let binding: Binding = disk::decode(&bytes)?;
        binding.validate(broker, &records, &quarantine)?;
        *pin = Some(Pin {
            binding,
            file: disk::FilePin::from_read(&bytes, identity),
        });
    }
    let pinned = pin.as_ref().ok_or(BrokerError::CorruptWriterState)?;
    pinned
        .file
        .check(&records, "binding.json", disk::IDENTITY_LIMIT)?;
    pinned.binding.validate(broker, &records, &quarantine)?;
    broker.writer.check(&broker.root, &broker.data)?;
    Ok(Some(Roots {
        records,
        quarantine,
        limits: pinned.binding.limits,
    }))
}

pub(crate) fn required(broker: &FsVaultBroker) -> Result<Roots, BrokerError> {
    open(broker)?.ok_or_else(|| BrokerError::InvalidArtifact("文件操作能力尚未显式启用".into()))
}

pub(crate) fn validate_state(
    broker: &FsVaultBroker,
    state: &crate::mutation::State,
) -> Result<(), BrokerError> {
    let roots = open(broker)?;
    if state.file_operations_enabled != roots.is_some() {
        return Err(BrokerError::CorruptWriterState);
    }
    if let Some(active) = &state.file_operation {
        let roots = roots.ok_or(BrokerError::CorruptWriterState)?;
        if active.frozen.vault_uuid != broker.vault_uuid || active.frozen.limits != roots.limits {
            return Err(BrokerError::CorruptWriterState);
        }
    }
    Ok(())
}
