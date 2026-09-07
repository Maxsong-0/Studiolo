//! 初始化是显式操作；普通重开既不造目录，也不补写缺失的配对记录。

use super::{Binding, DirectoryPin, Identity, Writer, disk, lease};
use crate::{BrokerError, FsVaultBroker, VaultIdentity, VaultUuid};
use cap_std::fs::{Dir, MetadataExt as _};
use std::{path::Path, sync::atomic::AtomicBool};

pub(super) fn open(
    vault_path: &Path,
    data_path: &Path,
    initialize: bool,
) -> Result<(Dir, Dir, VaultUuid, Writer), BrokerError> {
    let vault_path = lease::absolute(vault_path)?;
    let data_path = lease::absolute(data_path)?;
    if vault_path.starts_with(&data_path) || data_path.starts_with(&vault_path) {
        return Err(BrokerError::WriterBindingMismatch);
    }
    let root = lease::walk(&vault_path, initialize)?;
    let lease = lease::acquire(&root)?;
    let root_pin = DirectoryPin::new(vault_path.clone(), &root)?;
    root_pin.check()?;
    if lease::has_ancestor(&data_path, Identity::directory(&root)?)? {
        return Err(BrokerError::WriterBindingMismatch);
    }
    // 获取 canonical 租约后才接触 data，争用失败不能初始化另一份存储。
    ensure(&root, ".workbench", initialize)?;
    let control = lease::child(&root, ".workbench")?;
    let control_identity = Identity::directory(&control)?;
    let left = disk::read_identified(&control, "broker-binding.json", disk::IDENTITY_LIMIT)?;
    let left_pin = left
        .as_ref()
        .map(|(bytes, id)| disk::FilePin::from_read(bytes, *id));
    let data = lease::walk(&data_path, initialize && left.is_none())?;
    let data_pin = DirectoryPin::new(data_path, &data)?;
    if lease::has_ancestor(&vault_path, Identity::directory(&data)?)? {
        return Err(BrokerError::WriterBindingMismatch);
    }
    let check_roots = || {
        root_pin.check()?;
        data_pin.check()?;
        if Identity::directory(&lease::child(&root, ".workbench")?)? != control_identity {
            return Err(BrokerError::WriterBindingMismatch);
        }
        Ok(())
    };
    check_roots()?;
    let right = disk::read_identified(&data, "broker-binding.json", disk::IDENTITY_LIMIT)?;
    let right_pin = right
        .as_ref()
        .map(|(bytes, id)| disk::FilePin::from_read(bytes, *id));
    if left.is_some() != right.is_some() {
        return Err(BrokerError::WriterBindingMismatch);
    }
    let adopting = left.is_none();
    if adopting && !initialize {
        return Err(BrokerError::WriterBindingMismatch);
    }
    if adopting && disk::read(&data, "writer-state.json", disk::RECORD_LIMIT)?.is_some() {
        return Err(BrokerError::CorruptWriterState);
    }
    // 不合法旧记录被拒绝时，不能先写入 marker、staging 或新的绑定。
    let legacy_unresolved = if adopting {
        match data.symlink_metadata("idempotency") {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
                crate::mutation::validate_replays(&data, None)?
            }
            Ok(_) => return Err(BrokerError::CorruptWriterState),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(BrokerError::io("inspect legacy mutation store", e)),
        }
    } else {
        false
    };
    let (uuid, marker_pin) = identity(&control, adopting && initialize, check_roots)?;
    for name in ["history", "idempotency", "snapshots"] {
        check_roots()?;
        ensure(&data, name, adopting)?;
    }
    check_roots()?;
    ensure(&control, "staging", adopting)?;
    let binding = if let (Some((left, _)), Some((right, _))) = (left, right) {
        let left: Binding = disk::decode(&left)?;
        let right: Binding = disk::decode(&right)?;
        if left != right {
            return Err(BrokerError::WriterBindingMismatch);
        }
        left.verify(&root, &data, uuid)?;
        left
    } else {
        // 完整检查旧回执后才写配对；未知旧未决只作为阻断事实迁移。
        let binding = Binding::capture(&root, &data, uuid)?;
        let state = crate::mutation::State::initial(binding.pair_id, legacy_unresolved);
        let check = || {
            check_roots()?;
            marker_pin.check(&control, "vault.json", disk::IDENTITY_LIMIT)?;
            binding.verify(&root, &data, uuid)
        };
        disk::create(&control, "broker-binding.json", &binding, check)?;
        disk::create(&data, "broker-binding.json", &binding, check)?;
        disk::create(&data, "writer-state.json", &state, check)?;
        binding
    };
    let writer = Writer {
        _lease: lease,
        root: root_pin,
        data: data_pin,
        control: binding.control,
        marker: marker_pin,
        root_binding: match left_pin {
            Some(pin) => pin,
            None => disk::FilePin::capture(&control, "broker-binding.json", disk::IDENTITY_LIMIT)?,
        },
        data_binding: match right_pin {
            Some(pin) => pin,
            None => disk::FilePin::capture(&data, "broker-binding.json", disk::IDENTITY_LIMIT)?,
        },
        directories: vec![
            (true, ".workbench/staging", binding.staging),
            (false, "history", binding.history),
            (false, "idempotency", binding.idempotency),
            (false, "snapshots", binding.snapshots),
        ],
        binding,
        uncertain: AtomicBool::new(false),
        highest_committed: std::sync::atomic::AtomicU64::new(0),
        rollback: AtomicBool::new(false),
        #[cfg(test)]
        fault: std::sync::atomic::AtomicU8::new(0),
    };
    writer.check(&root, &data)?;
    Ok((root, data, uuid, writer))
}

fn ensure(parent: &Dir, name: &str, create: bool) -> Result<(), BrokerError> {
    match parent.symlink_metadata(name) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        Ok(_) => return Err(BrokerError::WriterBindingMismatch),
        Err(e) if create && e.kind() == std::io::ErrorKind::NotFound => {
            parent
                .create_dir(name)
                .map_err(|e| BrokerError::io("create writer layout", e))?;
            FsVaultBroker::sync_directory(parent, Path::new("."))?;
        }
        Err(e) => return Err(BrokerError::io("inspect writer layout", e)),
    }
    lease::child(parent, name)?;
    Ok(())
}

fn identity(
    control: &Dir,
    initialize: bool,
    check: impl Fn() -> Result<(), BrokerError>,
) -> Result<(VaultUuid, disk::FilePin), BrokerError> {
    // 保留既有 marker 非普通节点的 CorruptIdentity 契约；后续读取仍独立
    // 使用 nofollow 和同次 bytes/inode pin，前置检查不替代竞争检测。
    match control.symlink_metadata("vault.json") {
        Ok(metadata)
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.nlink() > 1 =>
        {
            return Err(BrokerError::CorruptIdentity);
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(BrokerError::io("inspect Vault identity", error)),
    }
    if let Some((bytes, id)) = disk::read_identified(control, "vault.json", disk::IDENTITY_LIMIT)? {
        return Ok((
            VaultIdentity::decode(&bytes)?,
            disk::FilePin::from_read(&bytes, id),
        ));
    }
    if !initialize {
        return Err(BrokerError::CorruptIdentity);
    }
    let identity = VaultIdentity {
        format_version: 1,
        vault_uuid: VaultUuid::new(),
    };
    check()?;
    disk::create(control, "vault.json", &identity, check)?;
    let (bytes, id) = disk::read_identified(control, "vault.json", disk::IDENTITY_LIMIT)?
        .ok_or(BrokerError::CorruptIdentity)?;
    if VaultIdentity::decode(&bytes)? != identity.vault_uuid {
        return Err(BrokerError::WriterBindingMismatch);
    }
    Ok((identity.vault_uuid, disk::FilePin::from_read(&bytes, id)))
}
