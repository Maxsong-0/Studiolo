//! 私有 snapshots 下的 recovery- 命名空间；普通 SnapshotId 无法寻址，不自动清理。

use super::PendingMutation;
use crate::mutation::record::Operation;
use crate::writer::{directory, disk};
use crate::{BrokerError, FsVaultBroker, Revision, VaultPath, VaultUuid, sha256};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    pair_id: VaultUuid,
    vault_uuid: VaultUuid,
    path: VaultPath,
    sequence: u64,
    fingerprint: String,
    target: Revision,
    previous: Option<Revision>,
    current: Option<Revision>,
    files: Vec<Object>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Object {
    role: String,
    revision: Revision,
    digest: String,
    bytes: usize,
}

pub(super) fn preserve(
    broker: &FsVaultBroker,
    review: &PendingMutation,
    proposed: super::proposed::Proposed,
) -> Result<String, BrokerError> {
    let op = review.operation();
    if proposed.identity().bytes != op.stage.size
        || proposed.identity().sha256 != op.stage.digest
        || proposed.revision()? != op.target_revision
    {
        return Err(BrokerError::IdempotencyConflict);
    }
    let mut contents = vec![("proposed", proposed)];
    if review.current.identity.is_some() {
        contents.push((
            "current",
            super::proposed::Proposed::stored(
                broker,
                broker
                    .root
                    .try_clone()
                    .map_err(|e| BrokerError::io("clone current evidence root", e))?,
                op.path.clone(),
                &op.path,
                review
                    .current_revision()
                    .ok_or(BrokerError::CorruptWriterState)?,
            )?,
        ));
    }
    if let Some(previous) = op.previous_revision {
        contents.push((
            "previous",
            super::proposed::Proposed::stored(
                broker,
                directory(&broker.data, "history")?,
                crate::blob::store::logical(&previous.to_string())?,
                &op.path,
                previous,
            )?,
        ));
    }
    let manifest = Manifest {
        // 旧小请求 Installing 期间 current 也可能被外部换成大对象。
        // 清单预算取本次实际保全对象，不以原请求日志版本推断对象大小。
        version: if review.state.format_version >= 4
            || contents
                .iter()
                .any(|(_, value)| value.identity().bytes > disk::CONTENT_LIMIT as u64)
        {
            2
        } else {
            1
        },
        pair_id: review.state.pair_id,
        vault_uuid: broker.vault_uuid,
        path: op.path.clone(),
        sequence: op.sequence,
        fingerprint: op.fingerprint.clone(),
        target: op.target_revision,
        previous: op.previous_revision,
        current: review.current_revision(),
        files: contents
            .iter()
            .map(|(role, value)| {
                Ok(Object {
                    role: (*role).to_owned(),
                    revision: value.revision()?,
                    digest: value.identity().sha256.clone(),
                    bytes: value.identity().bytes as usize,
                })
            })
            .collect::<Result<Vec<_>, BrokerError>>()?,
    };
    let encoded = serde_json::to_vec(&manifest).map_err(|_| BrokerError::CorruptWriterState)?;
    let id = sha256(&encoded);
    let dir = directory(&broker.data, "snapshots")?;
    for (role, value) in contents {
        value.preserve(broker, &dir, &format!("recovery-{id}-{role}.bin"), &op.path)?;
    }
    store(broker, &dir, &format!("recovery-{id}.json"), &encoded)?;
    verify(broker, &id, op, review.current_revision())?;
    Ok(id)
}

fn store(
    broker: &FsVaultBroker,
    dir: &cap_std::fs::Dir,
    name: &str,
    bytes: &[u8],
) -> Result<(), BrokerError> {
    if let Some(existing) = disk::read(dir, name, disk::CONTENT_LIMIT)? {
        if existing != bytes {
            return Err(BrokerError::CorruptWriterState);
        }
    } else {
        FsVaultBroker::atomic_write_dir_checked(dir, Path::new(name), bytes, true, || {
            broker.writer.check(&broker.root, &broker.data)
        })?;
    }
    let logical = VaultPath::parse(name).map_err(|_| BrokerError::CorruptWriterState)?;
    FsVaultBroker::open_rooted_file(dir, &logical)?
        .sync_all()
        .map_err(|e| BrokerError::io("sync reconciliation evidence", e))?;
    FsVaultBroker::sync_directory(dir, Path::new("."))?;
    broker.writer.check(&broker.root, &broker.data)
}

pub(super) fn verify(
    broker: &FsVaultBroker,
    id: &str,
    op: &Operation,
    current: Option<Revision>,
) -> Result<(), BrokerError> {
    verify_version(broker, id, op, current).map(|_| ())
}

/// 只有整份证据验证成功才返回闭集版本，供终态清理单调启用 State4。
pub(super) fn verify_version(
    broker: &FsVaultBroker,
    id: &str,
    op: &Operation,
    current: Option<Revision>,
) -> Result<u32, BrokerError> {
    if !disk::hex(id) {
        return Err(BrokerError::CorruptWriterState);
    }
    let dir = directory(&broker.data, "snapshots")?;
    let bytes = disk::read(&dir, &format!("recovery-{id}.json"), disk::RECORD_LIMIT)?
        .ok_or(BrokerError::CorruptWriterState)?;
    if sha256(&bytes) != id {
        return Err(BrokerError::CorruptWriterState);
    }
    let manifest: Manifest = disk::decode(&bytes)?;
    let roles: Vec<_> = manifest
        .files
        .iter()
        .map(|file| file.role.as_str())
        .collect();
    let mut expected = vec!["proposed"];
    if current.is_some() {
        expected.push("current");
    }
    if op.previous_revision.is_some() {
        expected.push("previous");
    }
    if !matches!(manifest.version, 1 | 2)
        || manifest.pair_id != broker.writer.binding.pair_id
        || manifest.vault_uuid != broker.vault_uuid
        || manifest.path != op.path
        || manifest.sequence != op.sequence
        || manifest.fingerprint != op.fingerprint
        || manifest.target != op.target_revision
        || manifest.previous != op.previous_revision
        || manifest.current != current
        || roles != expected
    {
        return Err(BrokerError::CorruptWriterState);
    }
    for file in manifest.files {
        let expected_revision = match file.role.as_str() {
            "proposed" => Some(op.target_revision),
            "current" => current,
            "previous" => op.previous_revision,
            _ => None,
        };
        let proof = crate::blob::store::verify(
            broker,
            &dir,
            &format!("recovery-{id}-{}.bin", file.role),
            &op.path,
            file.revision,
        )?;
        let cap = if manifest.version == 2 {
            crate::blob::OBJECT_LIMIT
        } else {
            disk::CONTENT_LIMIT as u64
        };
        if file.bytes as u64 > cap
            || proof.identity.bytes != file.bytes as u64
            || proof.identity.sha256 != file.digest
            || Some(file.revision) != expected_revision
        {
            return Err(BrokerError::CorruptWriterState);
        }
    }
    broker.writer.check(&broker.root, &broker.data)?;
    Ok(manifest.version)
}
