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
    // 第 1 版没有恢复标识；第 2 版必须有、而且合法；第 3 版再加之前用过的那一串（每个都合法、不重复、不含这次
    // 的、不超过上限）。别的版本、几版混写都按未知版本拒绝。
    let identified = lineage
        .restore_id
        .as_deref()
        .is_some_and(super::valid_restore_id);
    let consistent = match lineage.format_version {
        1 => lineage.restore_id.is_none() && lineage.earlier_restores.is_empty(),
        2 => identified && lineage.earlier_restores.is_empty(),
        RestoreLineage::FORMAT_VERSION => identified && consistent_chain(&lineage),
        _ => false,
    };
    if !consistent {
        return Err(RestoreError::UnsupportedLineage);
    }
    Ok(Some(lineage))
}

/// 之前用过的恢复标识那一串自洽：每个都合法、互不相同、不含这次的、不超过上限。
fn consistent_chain(lineage: &RestoreLineage) -> bool {
    let earlier = &lineage.earlier_restores;
    let distinct: std::collections::BTreeSet<&str> = earlier.iter().map(String::as_str).collect();
    earlier.len() <= RestoreLineage::MAX_EARLIER_RESTORES
        && distinct.len() == earlier.len()
        && earlier.iter().all(|id| super::valid_restore_id(id))
        && lineage
            .restore_id
            .as_ref()
            .is_none_or(|current| !earlier.contains(current))
}

pub(super) struct Rebound {
    pub(super) coverage: RebindCoverage,
    pub(super) lineage: RestoreLineage,
}

/// 新 lineage 里说明「这是哪一次恢复」的四项，原样写进 [`RestoreLineage`]。
pub(super) struct RestoreChain<'a> {
    /// 这次恢复基于的 checkpoint。
    pub(super) checkpoint_id: &'a str,
    /// 这次的恢复标识。
    pub(super) restore_id: &'a str,
    /// 上一环：被封存的根本身是恢复出来的，就是它的恢复标识。
    pub(super) parent_lineage: Option<String>,
    /// 这份数据以前用过的全部恢复标识，按先后（[`RestoreLineage::earlier_for_next`]）。
    pub(super) earlier_restores: Vec<String>,
}

#[allow(clippy::too_many_lines)]
pub(super) fn rebind(
    root: &Dir,
    data: &Dir,
    verified: &Verified,
    chain: RestoreChain<'_>,
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
        checkpoint_id: chain.checkpoint_id.to_owned(),
        previous_canonical: previous.0.into(),
        previous_data: previous.1.into(),
        restored_canonical: None,
        restored_data: None,
        parent_lineage: chain.parent_lineage,
        restore_id: Some(chain.restore_id.to_owned()),
        earlier_restores: chain.earlier_restores,
    };
    write_lineage(data, &control, &lineage)?;
    fault.trip(RestoreFault::AfterLineage)?;

    let history = if let Some(previous) = &verified.history {
        let history_root = directory(data, "history").map_err(broker)?;
        let meta = directory(&history_root, HISTORY_META).map_err(broker)?;
        let history = super::mirror::HistoryBinding {
            version: previous.version,
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
        super::BindingCoverage::Rebound
    } else {
        super::BindingCoverage::NotEnabled
    };
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

    let file_operations = if let Some(previous) = &verified.file_operations {
        super::optional::rebind_file_operations(root, data, previous, || {
            marker_check(&control, verified.vault_uuid)
        })?;
        super::BindingCoverage::Rebound
    } else {
        super::BindingCoverage::NotEnabled
    };
    fault.trip(RestoreFault::AfterFileOperations)?;

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
            history_meta_v1: history,
            file_operations_v1: file_operations,
            maintenance_v1: maintenance,
        },
        lineage,
    })
}
