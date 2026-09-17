//! 完整 Idle 先发布，再在主日志启用；启用后缺槽或替换绑定必须失败关闭。
use super::record::Slot;
use crate::{
    BrokerError, FsVaultBroker, VaultUuid, mutation,
    writer::{Identity, directory, disk},
};
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use std::path::Path;
const NAME: &str = "maintenance-v1";
const PREPARING: &str = "maintenance-v1.preparing";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    version: u32,
    pair_id: VaultUuid,
    vault_uuid: VaultUuid,
    root: Identity,
}
pub(crate) struct Pin {
    binding: Binding,
    file: disk::FilePin,
    slot: disk::FilePin,
}

fn open(broker: &FsVaultBroker) -> Result<Option<Dir>, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    match broker.data.symlink_metadata(PREPARING) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(BrokerError::io("inspect maintenance preparation", e)),
        Ok(_) => return Err(BrokerError::CorruptWriterState),
    }
    let mut pin = broker
        .maintenance_pin
        .lock()
        .map_err(|_| BrokerError::CorruptWriterState)?;
    let dir = match broker.data.symlink_metadata(NAME) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if pin.is_some() {
                return Err(BrokerError::WriterBindingMismatch);
            }
            return Ok(None);
        }
        Err(e) => return Err(BrokerError::io("inspect maintenance schema", e)),
        Ok(_) => directory(&broker.data, NAME)?,
    };
    if pin.is_none() {
        let (bytes, identity) = disk::read_identified(&dir, "binding.json", disk::IDENTITY_LIMIT)?
            .ok_or(BrokerError::CorruptWriterState)?;
        let binding: Binding = disk::decode(&bytes)?;
        validate(broker, &dir, &binding)?;
        *pin = Some(Pin {
            binding,
            file: disk::FilePin::from_read(&bytes, identity),
            slot: disk::FilePin::capture(&dir, "slot.json", disk::RECORD_LIMIT)?,
        });
    }
    let pin = pin.as_ref().ok_or(BrokerError::CorruptWriterState)?;
    validate(broker, &dir, &pin.binding)?;
    pin.file.check(&dir, "binding.json", disk::IDENTITY_LIMIT)?;
    pin.slot.check(&dir, "slot.json", disk::RECORD_LIMIT)?;
    broker.writer.check(&broker.root, &broker.data)?;
    Ok(Some(dir))
}
fn validate(broker: &FsVaultBroker, dir: &Dir, binding: &Binding) -> Result<(), BrokerError> {
    if binding.version != 1
        || binding.pair_id != broker.writer.binding.pair_id
        || binding.vault_uuid != broker.vault_uuid
        || binding.root != Identity::directory(dir)?
    {
        Err(BrokerError::WriterBindingMismatch)
    } else {
        Ok(())
    }
}
pub(super) fn load(broker: &FsVaultBroker) -> Result<Option<Slot>, BrokerError> {
    let state = mutation::load(broker)?;
    if broker.writer.uncertain() {
        return Err(BrokerError::MutationUnresolved);
    }
    let Some(dir) = open(broker)? else {
        return if state.maintenance_enabled {
            Err(BrokerError::CorruptWriterState)
        } else {
            Ok(None)
        };
    };
    let slot: Slot = disk::decode(
        &disk::read(&dir, "slot.json", disk::RECORD_LIMIT)?
            .ok_or(BrokerError::CorruptWriterState)?,
    )?;
    slot.validate(broker)?;
    if !state.maintenance_enabled && slot.generation != 0 {
        return Err(BrokerError::CorruptWriterState);
    }
    open(broker)?;
    Ok(Some(slot))
}
pub(super) fn enable(broker: &FsVaultBroker) -> Result<Slot, BrokerError> {
    if load(broker)?.is_none() {
        initialize(broker)?;
    }
    mutation::fault(broker, 30)?;
    let slot = load(broker)?.ok_or(BrokerError::CorruptWriterState)?;
    let mut state = mutation::load(broker)?;
    if !state.maintenance_enabled {
        state.maintenance_enabled = true;
        state.format_version = state.format_version.max(3);
        mutation::save(broker, &state)?;
    }
    mutation::fault(broker, 31)?;
    Ok(slot)
}
pub(super) fn save(broker: &FsVaultBroker, slot: &Slot) -> Result<(), BrokerError> {
    slot.validate(broker)?;
    let dir = open(broker)?.ok_or(BrokerError::CorruptWriterState)?;
    let result = disk::save(&dir, "slot.json", slot, || {
        open(broker)?.ok_or(BrokerError::CorruptWriterState)?;
        Ok(())
    });
    if let Err(error) = result {
        broker.writer.poison();
        return Err(error);
    }
    let pin = disk::FilePin::capture(&dir, "slot.json", disk::RECORD_LIMIT)?;
    broker
        .maintenance_pin
        .lock()
        .map_err(|_| BrokerError::CorruptWriterState)?
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?
        .slot = pin;
    open(broker)?;
    Ok(())
}
fn initialize(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    broker
        .data
        .create_dir(PREPARING)
        .map_err(|e| BrokerError::io("prepare maintenance schema", e))?;
    let dir = directory(&broker.data, PREPARING)?;
    let binding = Binding {
        version: 1,
        pair_id: broker.writer.binding.pair_id,
        vault_uuid: broker.vault_uuid,
        root: Identity::directory(&dir)?,
    };
    disk::create(&dir, "binding.json", &binding, || {
        broker.writer.check(&broker.root, &broker.data)
    })?;
    disk::create(
        &dir,
        "slot.json",
        &Slot {
            version: 1,
            pair_id: binding.pair_id,
            generation: 0,
            record: None,
        },
        || broker.writer.check(&broker.root, &broker.data),
    )?;
    FsVaultBroker::sync_directory(&dir, Path::new("."))?;
    broker.writer.check(&broker.root, &broker.data)?;
    broker
        .data
        .rename(PREPARING, &broker.data, NAME)
        .map_err(|e| BrokerError::io("publish maintenance schema", e))?;
    FsVaultBroker::sync_directory(&broker.data, Path::new("."))?;
    open(broker)?.ok_or(BrokerError::CorruptWriterState)?;
    Ok(())
}
