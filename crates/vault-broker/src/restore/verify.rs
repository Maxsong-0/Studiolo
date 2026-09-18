//! 重绑定之前先完整验证封存内容与所有逻辑引用。
//!
//! 验证不改任何字节：确认 marker、两处主 binding、写者持久记录、全部幂等回执、
//! `history/meta-v1` 与已启用的 `maintenance-v1` 都指向同一个逻辑 `pair_id`，
//! 且没有半初始化残留，才允许进入物理重绑定。

use super::RestoreError;
use super::mirror::{
    BINDING, BROKER_BINDING, CONTROL, HISTORY_META, HistoryBinding, MAINTENANCE,
    MAINTENANCE_PREPARING, MaintenanceBinding, WRITER_STATE,
};
use crate::writer::{Binding, directory, disk};
use crate::{BrokerError, VaultIdentity, VaultUuid};
use cap_std::fs::Dir;

/// 验证通过后带出的事实。
pub(super) struct Verified {
    pub(super) vault_uuid: VaultUuid,
    pub(super) pair_id: VaultUuid,
    pub(super) history: HistoryBinding,
    pub(super) maintenance: Option<MaintenanceBinding>,
}

fn reference(name: &'static str) -> RestoreError {
    RestoreError::LogicalReferenceMismatch { reference: name }
}

fn present(parent: &Dir, name: &str) -> Result<bool, RestoreError> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(RestoreError::NewRootIncomplete),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(RestoreError::Broker(BrokerError::io(
            "inspect restored layout",
            error,
        ))),
    }
}

fn require_directory(parent: &Dir, name: &str) -> Result<Dir, RestoreError> {
    if !present(parent, name)? {
        return Err(RestoreError::NewRootIncomplete);
    }
    directory(parent, name).map_err(RestoreError::Broker)
}

/// `resuming` 只在明确接手上一次中途失败的新目录时为真。
///
/// 中途失败可能停在「data 侧主 binding 已改写、canonical 侧仍是旧身份」这一刻，
/// 此时两处字节必然不同。接手时允许这种**只差物理身份**的不一致，
/// 但逻辑身份（格式版本、`pair_id`、Vault UUID）仍必须完全一致，
/// 否则依旧判为两份不同的存储被拼在一起。
pub(super) fn verify(
    root: &Dir,
    data: &Dir,
    expected_vault: VaultUuid,
    expected_pair: VaultUuid,
    resuming: bool,
) -> Result<Verified, RestoreError> {
    let control = require_directory(root, CONTROL)?;
    let marker = disk::read(&control, "vault.json", disk::IDENTITY_LIMIT)
        .map_err(RestoreError::Broker)?
        .ok_or(RestoreError::NewRootIncomplete)?;
    if VaultIdentity::decode(&marker).map_err(RestoreError::Broker)? != expected_vault {
        return Err(RestoreError::VaultIdentityMismatch);
    }

    let left = disk::read(&control, BROKER_BINDING, disk::IDENTITY_LIMIT)
        .map_err(RestoreError::Broker)?
        .ok_or(RestoreError::NewRootIncomplete)?;
    let right = disk::read(data, BROKER_BINDING, disk::IDENTITY_LIMIT)
        .map_err(RestoreError::Broker)?
        .ok_or(RestoreError::NewRootIncomplete)?;
    let left: Binding = disk::decode(&left).map_err(RestoreError::Broker)?;
    let right: Binding = disk::decode(&right).map_err(RestoreError::Broker)?;
    if left != right {
        let only_physical = resuming
            && left.format_version == right.format_version
            && left.pair_id == right.pair_id
            && left.vault_uuid == right.vault_uuid;
        if !only_physical {
            return Err(RestoreError::BindingsDiverged);
        }
    }
    if left.vault_uuid != expected_vault {
        return Err(RestoreError::VaultIdentityMismatch);
    }
    if left.pair_id != expected_pair {
        return Err(RestoreError::PairIdMismatch);
    }

    // 逻辑 pair_id 还被写者日志、幂等终态、历史与维护目录引用，逐项核对。
    let state = disk::read(data, WRITER_STATE, disk::RECORD_LIMIT)
        .map_err(RestoreError::Broker)?
        .ok_or(RestoreError::NewRootIncomplete)?;
    let state: crate::mutation::State = disk::decode(&state).map_err(RestoreError::Broker)?;
    if state.pair_id != expected_pair {
        return Err(reference("writer-state.json"));
    }
    crate::mutation::validate_replays(data, Some(expected_pair))
        .map_err(|_| reference("idempotency"))?;

    for name in ["idempotency", "snapshots"] {
        require_directory(data, name)?;
    }
    require_directory(&control, "staging")?;

    let history_root = require_directory(data, "history")?;
    let meta = require_directory(&history_root, HISTORY_META)?;
    let history_bytes = disk::read(&meta, BINDING, disk::IDENTITY_LIMIT)
        .map_err(RestoreError::Broker)?
        .ok_or(RestoreError::NewRootIncomplete)?;
    let history: HistoryBinding = disk::decode(&history_bytes).map_err(RestoreError::Broker)?;
    if history.version != 1 || history.vault_uuid != expected_vault {
        return Err(reference("history/meta-v1"));
    }
    if history.pair_id != expected_pair {
        return Err(RestoreError::PairIdMismatch);
    }
    for name in ["requests", "catalogs", "outbox"] {
        require_directory(&meta, name)?;
    }

    if present(data, MAINTENANCE_PREPARING)? {
        return Err(RestoreError::MaintenanceHalfInitialized);
    }
    let maintenance = if present(data, MAINTENANCE)? {
        let dir = directory(data, MAINTENANCE).map_err(RestoreError::Broker)?;
        let bytes = disk::read(&dir, BINDING, disk::IDENTITY_LIMIT)
            .map_err(RestoreError::Broker)?
            .ok_or(RestoreError::NewRootIncomplete)?;
        let binding: MaintenanceBinding = disk::decode(&bytes).map_err(RestoreError::Broker)?;
        if binding.version != 1 || binding.vault_uuid != expected_vault {
            return Err(reference("maintenance-v1"));
        }
        if binding.pair_id != expected_pair {
            return Err(RestoreError::PairIdMismatch);
        }
        if disk::read(&dir, "slot.json", disk::RECORD_LIMIT)
            .map_err(RestoreError::Broker)?
            .is_none()
        {
            return Err(reference("maintenance-v1/slot.json"));
        }
        Some(binding)
    } else {
        None
    };

    Ok(Verified {
        vault_uuid: expected_vault,
        pair_id: expected_pair,
        history,
        maintenance,
    })
}
