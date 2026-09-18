//! 物理重绑定序列。
//!
//! 顺序是刻意设计的：canonical 侧的主 binding **最后**才改写。
//! 在它被改写之前，两处 binding 不一致或仍指向旧设备号，
//! `open_existing` 一律以 `WriterBindingMismatch` 拒绝，
//! 所以任何中途失败都只留下隔离且不能服务的新目录，不会发布半配对活动根。

use super::lineage::{LINEAGE_DIR, LINEAGE_PATH, LineageState, RestoreLineage};
use super::mirror::{BINDING, BROKER_BINDING, CONTROL, HISTORY_META, MAINTENANCE};
use super::verify::Verified;
use super::{MaintenanceCoverage, RebindCoverage, RestoreError, RestoreFault};
use crate::writer::{Binding, Identity, directory, disk};
use crate::{BrokerError, FsVaultBroker, VaultIdentity};
use cap_std::fs::Dir;
use std::path::Path;

fn broker(error: BrokerError) -> RestoreError {
    RestoreError::Broker(error)
}

/// 每次落盘之前复核 marker 仍然是同一个已验证 Vault 身份。
fn marker_check(control: &Dir, expected: crate::VaultUuid) -> Result<(), BrokerError> {
    let bytes = disk::read(control, "vault.json", disk::IDENTITY_LIMIT)?
        .ok_or(BrokerError::CorruptIdentity)?;
    if VaultIdentity::decode(&bytes)? != expected {
        return Err(BrokerError::WriterBindingMismatch);
    }
    Ok(())
}

fn ensure_lineage_dir(data: &Dir) -> Result<(), RestoreError> {
    match data.symlink_metadata(LINEAGE_DIR) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(RestoreError::NewRootIncomplete),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            data.create_dir(LINEAGE_DIR)
                .map_err(|error| broker(BrokerError::io("create restore lineage", error)))?;
            FsVaultBroker::sync_directory(data, Path::new(".")).map_err(broker)
        }
        Err(error) => Err(broker(BrokerError::io("inspect restore lineage", error))),
    }
}

fn write_lineage(data: &Dir, control: &Dir, lineage: &RestoreLineage) -> Result<(), RestoreError> {
    let expected = lineage.vault_uuid;
    if disk::read(data, LINEAGE_PATH, disk::RECORD_LIMIT)
        .map_err(broker)?
        .is_some()
    {
        disk::save(data, LINEAGE_PATH, lineage, || {
            marker_check(control, expected)
        })
        .map_err(broker)
    } else {
        disk::create(data, LINEAGE_PATH, lineage, || {
            marker_check(control, expected)
        })
        .map_err(broker)
    }
}

/// 读取新 data 根上的 lineage；从未恢复过的目录返回 `None` 而不是报错。
pub(super) fn read_lineage(data: &Dir) -> Result<Option<RestoreLineage>, RestoreError> {
    match data.symlink_metadata(LINEAGE_DIR) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err(RestoreError::NewRootIncomplete),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(broker(BrokerError::io("inspect restore lineage", error))),
    }
    let Some(bytes) = disk::read(data, LINEAGE_PATH, disk::RECORD_LIMIT).map_err(broker)? else {
        return Ok(None);
    };
    let lineage: RestoreLineage = disk::decode(&bytes).map_err(broker)?;
    if lineage.format_version != RestoreLineage::FORMAT_VERSION {
        return Err(RestoreError::UnsupportedLineage);
    }
    Ok(Some(lineage))
}

pub(super) struct Rebound {
    pub(super) coverage: RebindCoverage,
    pub(super) lineage: RestoreLineage,
}

#[allow(clippy::too_many_lines)]
pub(super) fn rebind(
    root: &Dir,
    data: &Dir,
    verified: &Verified,
    checkpoint_id: &str,
    parent_lineage: Option<String>,
    fault: RestoreFault,
) -> Result<Rebound, RestoreError> {
    let control = directory(root, CONTROL).map_err(broker)?;
    // 记录恢复前的物理身份，lineage 才能说明「从哪一对目录来的」。
    let previous = {
        let bytes = disk::read(data, BROKER_BINDING, disk::IDENTITY_LIMIT)
            .map_err(broker)?
            .ok_or(RestoreError::NewRootIncomplete)?;
        let binding: Binding = disk::decode(&bytes).map_err(broker)?;
        (binding.canonical, binding.data)
    };

    ensure_lineage_dir(data)?;
    let mut lineage = RestoreLineage {
        format_version: RestoreLineage::FORMAT_VERSION,
        state: LineageState::Rebinding,
        vault_uuid: verified.vault_uuid,
        pair_id: verified.pair_id,
        checkpoint_id: checkpoint_id.to_owned(),
        previous_canonical: previous.0.into(),
        previous_data: previous.1.into(),
        restored_canonical: None,
        restored_data: None,
        parent_lineage,
    };
    write_lineage(data, &control, &lineage)?;
    fault.trip(RestoreFault::AfterLineage)?;

    // history/meta-v1：目录组的四个身份全部重新采集。
    let history_root = directory(data, "history").map_err(broker)?;
    let meta = directory(&history_root, HISTORY_META).map_err(broker)?;
    let history = super::mirror::HistoryBinding {
        version: verified.history.version,
        pair_id: verified.pair_id,
        vault_uuid: verified.vault_uuid,
        root: Identity::directory(&meta).map_err(broker)?,
        requests: Identity::directory(&directory(&meta, "requests").map_err(broker)?)
            .map_err(broker)?,
        catalogs: Identity::directory(&directory(&meta, "catalogs").map_err(broker)?)
            .map_err(broker)?,
        outbox: Identity::directory(&directory(&meta, "outbox").map_err(broker)?)
            .map_err(broker)?,
    };
    disk::save(&meta, BINDING, &history, || {
        marker_check(&control, verified.vault_uuid)
    })
    .map_err(broker)?;
    fault.trip(RestoreFault::AfterHistory)?;

    // 已启用的 maintenance-v1 同样要重绑定，不能只改主根。
    let maintenance = if let Some(existing) = &verified.maintenance {
        let dir = directory(data, MAINTENANCE).map_err(broker)?;
        let rebound = super::mirror::MaintenanceBinding {
            version: existing.version,
            pair_id: verified.pair_id,
            vault_uuid: verified.vault_uuid,
            root: Identity::directory(&dir).map_err(broker)?,
        };
        disk::save(&dir, BINDING, &rebound, || {
            marker_check(&control, verified.vault_uuid)
        })
        .map_err(broker)?;
        MaintenanceCoverage::Rebound
    } else {
        MaintenanceCoverage::NotEnabled
    };
    fault.trip(RestoreFault::AfterMaintenance)?;

    // 两处主 binding：先 data 侧，再 canonical 侧。
    let mut binding = Binding::capture(root, data, verified.vault_uuid).map_err(broker)?;
    binding.pair_id = verified.pair_id;
    disk::save(data, BROKER_BINDING, &binding, || {
        marker_check(&control, verified.vault_uuid)
    })
    .map_err(broker)?;
    fault.trip(RestoreFault::AfterDataBinding)?;

    disk::save(&control, BROKER_BINDING, &binding, || {
        marker_check(&control, verified.vault_uuid)
    })
    .map_err(broker)?;
    fault.trip(RestoreFault::AfterCanonicalBinding)?;

    binding
        .verify(root, data, verified.vault_uuid)
        .map_err(broker)?;

    lineage.state = LineageState::Completed;
    lineage.restored_canonical = Some(binding.canonical.into());
    lineage.restored_data = Some(binding.data.into());
    write_lineage(data, &control, &lineage)?;

    Ok(Rebound {
        coverage: RebindCoverage {
            canonical_binding: true,
            data_binding: true,
            history_meta_v1: true,
            maintenance_v1: maintenance,
        },
        lineage,
    })
}
