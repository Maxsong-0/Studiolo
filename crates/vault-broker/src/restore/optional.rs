//! 未启用和缺失严格区分；文件操作只重绑目录，历史事件与内容证明原字节不改。
use super::{
    RestoreError,
    mirror::{BINDING, HISTORY_META, HistoryBinding},
};
use crate::{
    BrokerError, VaultUuid,
    file_operation::binding,
    mutation::State,
    writer::{Identity, directory, disk},
};
use cap_std::fs::Dir;

pub(super) fn present(root: &Dir, name: &str) -> Result<bool, RestoreError> {
    match root.symlink_metadata(name) {
        Ok(value) if value.is_dir() && !value.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(RestoreError::NewRootIncomplete),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(RestoreError::Broker(BrokerError::io("核对恢复目录", e))),
    }
}
fn empty(root: &Dir) -> Result<bool, RestoreError> {
    Ok(root
        .entries()
        .map_err(|e| RestoreError::Broker(BrokerError::io("核对未启用历史", e)))?
        .next()
        .transpose()
        .map_err(|e| RestoreError::Broker(BrokerError::io("核对历史目录项", e)))?
        .is_none())
}
pub(super) fn history(
    data: &Dir,
    state: &State,
    vault: VaultUuid,
    pair: VaultUuid,
) -> Result<Option<HistoryBinding>, RestoreError> {
    let history = directory(data, "history").map_err(RestoreError::Broker)?;
    if !present(&history, HISTORY_META)? {
        if state.committed_through != 0
            || state.next_sequence != 1
            || state.legacy_unresolved
            || state.operation.is_some()
            || state.file_operation.is_some()
            || state.batch.is_some()
            || !empty(&history)?
            || !empty(&directory(data, "idempotency").map_err(RestoreError::Broker)?)?
        {
            return Err(RestoreError::NewRootIncomplete);
        }
        return Ok(None);
    }
    let meta = directory(&history, HISTORY_META).map_err(RestoreError::Broker)?;
    let value: HistoryBinding = disk::decode(
        &disk::read(&meta, BINDING, disk::IDENTITY_LIMIT)
            .map_err(RestoreError::Broker)?
            .ok_or(RestoreError::NewRootIncomplete)?,
    )
    .map_err(RestoreError::Broker)?;
    if value.version != 1 || value.vault_uuid != vault {
        return Err(RestoreError::LogicalReferenceMismatch {
            reference: "history/meta-v1",
        });
    }
    if value.pair_id != pair {
        return Err(RestoreError::PairIdMismatch);
    }
    for name in ["requests", "catalogs", "outbox"] {
        directory(&meta, name).map_err(RestoreError::Broker)?;
    }
    Ok(Some(value))
}
pub(super) fn file_operations(
    root: &Dir,
    data: &Dir,
    state: &State,
    vault: VaultUuid,
    pair: VaultUuid,
) -> Result<Option<binding::Binding>, RestoreError> {
    if present(data, binding::PREPARING)? || present(root, binding::QUARANTINE_PREPARING)? {
        return Err(RestoreError::NewRootIncomplete);
    }
    let records = present(data, binding::NAME)?;
    let quarantine = present(root, binding::QUARANTINE)?;
    if records != quarantine || records != state.file_operations_enabled {
        return Err(RestoreError::NewRootIncomplete);
    }
    if !records {
        return Ok(None);
    }
    let records = directory(data, binding::NAME).map_err(RestoreError::Broker)?;
    let value: binding::Binding = disk::decode(
        &disk::read(&records, BINDING, disk::IDENTITY_LIMIT)
            .map_err(RestoreError::Broker)?
            .ok_or(RestoreError::NewRootIncomplete)?,
    )
    .map_err(RestoreError::Broker)?;
    value.limits.validate().map_err(RestoreError::Broker)?;
    if value.version != 1
        || value.vault_uuid != vault
        || value.pair_id != pair
        || value.children.len() != binding::CHILDREN.len()
    {
        return Err(RestoreError::LogicalReferenceMismatch {
            reference: "file-operations-v1",
        });
    }
    for ((name, _), expected) in value.children.iter().zip(binding::CHILDREN) {
        if name != expected {
            return Err(RestoreError::NewRootIncomplete);
        }
        directory(&records, name).map_err(RestoreError::Broker)?;
    }
    Ok(Some(value))
}
pub(super) fn rebind_file_operations(
    root: &Dir,
    data: &Dir,
    previous: &binding::Binding,
    check: impl Fn() -> Result<(), BrokerError>,
) -> Result<(), RestoreError> {
    let records = directory(data, binding::NAME).map_err(RestoreError::Broker)?;
    let mut value = previous.clone();
    value.root = Identity::directory(&records).map_err(RestoreError::Broker)?;
    value.quarantine =
        Identity::directory(&directory(root, binding::QUARANTINE).map_err(RestoreError::Broker)?)
            .map_err(RestoreError::Broker)?;
    if value.quarantine.dev != Identity::directory(root).map_err(RestoreError::Broker)?.dev {
        return Err(RestoreError::NewRootIncomplete);
    }
    for (name, identity) in &mut value.children {
        *identity = Identity::directory(&directory(&records, name).map_err(RestoreError::Broker)?)
            .map_err(RestoreError::Broker)?;
    }
    disk::save(&records, BINDING, &value, check).map_err(RestoreError::Broker)
}
