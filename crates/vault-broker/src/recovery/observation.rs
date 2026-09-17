//! 观察完整字节和文件/父目录 ctime，拒绝审阅后的同内容替换或 ABA。

use super::PendingMutation;
use crate::writer::{Identity, directory, disk};
use crate::{BrokerError, FsVaultBroker, VaultPath};
use cap_std::fs::{Dir, Metadata, MetadataExt as _};
use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct Stamp {
    identity: Identity,
    ctime: i64,
    nanos: i64,
    size: u64,
}

impl Stamp {
    fn new(metadata: &Metadata) -> Self {
        Self {
            identity: Identity::file(metadata),
            ctime: metadata.ctime(),
            nanos: metadata.ctime_nsec(),
            size: metadata.len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Observed {
    pub(super) bytes: Option<Vec<u8>>,
    pub(super) identity: Option<crate::BlobIdentity>,
    pub(super) revision: Option<crate::Revision>,
    parent: Stamp,
    file: Option<Stamp>,
}

impl Observed {
    pub(super) fn maintenance_digest(&self) -> Result<String, BrokerError> {
        Ok(crate::sha256(
            &serde_json::to_vec(&(
                self.identity.as_ref().map(|value| &value.sha256),
                &self.parent,
                &self.file,
            ))
            .map_err(|_| BrokerError::CorruptWriterState)?,
        ))
    }
    pub(super) fn same_file(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.file == other.file
            && self.parent.identity == other.parent.identity
    }
}

pub(super) fn capture(dir: &Dir, path: &str, limit: usize) -> Result<Observed, BrokerError> {
    capture_bound(dir, path, limit, None)
}
pub(super) fn current(broker: &FsVaultBroker, path: &VaultPath) -> Result<Observed, BrokerError> {
    capture_bound(
        &broker.root,
        path.as_str(),
        crate::blob::OBJECT_LIMIT as usize,
        Some((broker.vault_uuid, path)),
    )
}
fn capture_bound(
    dir: &Dir,
    path: &str,
    limit: usize,
    binding: Option<(crate::VaultUuid, &VaultPath)>,
) -> Result<Observed, BrokerError> {
    let path = Path::new(path);
    let parent_path = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent_path = parent_path
        .to_str()
        .ok_or(BrokerError::CorruptWriterState)?;
    let parent = directory(dir, parent_path)?;
    let parent_stamp = Stamp::new(
        &parent
            .dir_metadata()
            .map_err(|e| BrokerError::io("observe parent", e))?,
    );
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(BrokerError::CorruptWriterState)?;
    let logical = VaultPath::parse(name).map_err(|_| BrokerError::CorruptWriterState)?;
    let before = stat(&parent, &logical)?;
    let proof = match crate::blob::file::scan(&parent, &logical, limit as u64, binding) {
        Ok(proof) => Some(proof),
        Err(BrokerError::NotFound(_)) => None,
        Err(error) => return Err(error),
    };
    // 只有有界 journal 需要字节反序列化；正文/附件始终只保留固定摘要和身份。
    let bytes = if limit <= disk::RECORD_LIMIT {
        disk::read(&parent, name, limit)?
    } else {
        None
    };
    let identity = proof.as_ref().map(|value| value.identity.clone());
    let revision = proof.and_then(|value| value.revision);
    let after = stat(&parent, &logical)?;
    let current_parent = directory(dir, parent_path)?;
    let current_stamp = Stamp::new(
        &current_parent
            .dir_metadata()
            .map_err(|e| BrokerError::io("verify parent", e))?,
    );
    if parent_stamp != current_stamp || before != after || before.is_some() != identity.is_some() {
        return Err(BrokerError::CaptureChanged);
    }
    Ok(Observed {
        bytes,
        identity,
        revision,
        parent: parent_stamp,
        file: after,
    })
}

fn stat(parent: &Dir, path: &VaultPath) -> Result<Option<Stamp>, BrokerError> {
    let file = match FsVaultBroker::open_rooted_file(parent, path) {
        Ok(file) => file,
        Err(BrokerError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file
        .metadata()
        .map_err(|e| BrokerError::io("observe file", e))?;
    FsVaultBroker::validate_file_metadata(path, &metadata)?;
    Ok(Some(Stamp::new(&metadata)))
}

impl FsVaultBroker {
    /// 返回最新一次审阅票据；新审阅使本实例所有旧票据失效，不写文件或恢复日志。
    pub fn pending_mutation(&self) -> Result<Option<PendingMutation>, BrokerError> {
        let ticket = self
            .recovery_review
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| BrokerError::WriterLimitExceeded)?
            + 1;
        let state = crate::mutation::load(self)?;
        if state.legacy_unresolved || self.writer.uncertain() {
            return Err(BrokerError::MutationUnresolved);
        }
        let Some(operation) = state.operation.clone() else {
            return Ok(None);
        };
        if operation.phase == crate::mutation::record::Phase::Completed {
            return Err(BrokerError::MutationUnresolved);
        }
        self.validate_components(&operation.path, true)?;
        let journal = capture(&self.data, "writer-state.json", disk::RECORD_LIMIT)?;
        let current = current(self, &operation.path)?;
        let stage = capture(
            &self.root,
            &format!(".workbench/staging/{}", operation.stage.name),
            crate::blob::OBJECT_LIMIT as usize,
        )?;
        let original = serde_json::to_vec(&state).map_err(|_| BrokerError::CorruptWriterState)?;
        let observed: crate::mutation::State = disk::decode(
            journal
                .bytes
                .as_deref()
                .ok_or(BrokerError::CorruptWriterState)?,
        )?;
        if original != serde_json::to_vec(&observed).map_err(|_| BrokerError::CorruptWriterState)? {
            return Err(BrokerError::CaptureChanged);
        }
        self.writer.check(&self.root, &self.data)?;
        Ok(Some(PendingMutation {
            instance: Arc::clone(&self.capture_instance),
            ticket,
            state,
            operation,
            vault_uuid: self.vault_uuid,
            journal,
            current,
            stage,
        }))
    }
}

pub(super) fn check(broker: &FsVaultBroker, review: &PendingMutation) -> Result<(), BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    if !Arc::ptr_eq(&broker.capture_instance, &review.instance)
        || broker.recovery_review.load(Ordering::Acquire) != review.ticket
        || broker.writer.uncertain()
        || capture(&broker.data, "writer-state.json", disk::RECORD_LIMIT)? != review.journal
        || current(broker, review.path())? != review.current
        || capture(
            &broker.root,
            &format!(".workbench/staging/{}", review.operation().stage.name),
            crate::blob::OBJECT_LIMIT as usize,
        )? != review.stage
    {
        return Err(BrokerError::CaptureChanged);
    }
    broker.writer.check(&broker.root, &broker.data)
}
