//! 基于文件系统的 canonical Vault 适配器：结构体、打开、稳定读取与快照。

mod anchors;
mod artifact;
mod components;
mod rooted;

pub(crate) use components::collision_component_key;

use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, atomic::AtomicU64},
};

use cap_std::fs::{Dir, MetadataExt as _};
use serde::Serialize;
use studiolo_core::{AttemptId, Revision, TaskId, VaultPath, VaultUuid};

use crate::{
    BrokerError, Snapshot, SnapshotFile, SnapshotId, SnapshotInput, SnapshotReader, VaultObject,
    VaultReader, VaultSnapshotter, capture, file_operation, history, maintenance,
    memory_state_error, sha256, types::snapshot_identity, writer,
};

/// 基于文件系统的 canonical Vault 适配器。
pub struct FsVaultBroker {
    pub(crate) root: Dir,
    pub(crate) data: Dir,
    pub(crate) vault_uuid: VaultUuid,
    pub(crate) writer: writer::Writer,
    pub(crate) capture_instance: std::sync::Arc<()>,
    pub(crate) recovery_review: AtomicU64,
    pub(crate) history_pin: Mutex<Option<history::store::Pin>>,
    pub(crate) maintenance_pin: Mutex<Option<maintenance::store::Pin>>,
    pub(crate) file_operation_pin: std::sync::Mutex<Option<file_operation::binding::Pin>>,
    pub(crate) read_guard: Mutex<()>,
}

impl FsVaultBroker {
    /// 显式初始化或采用经过验证的旧存储；普通写者重开应使用 `open_existing`。
    pub fn open(vault_root: &Path, data_root: &Path) -> Result<Self, BrokerError> {
        Self::open_writer(vault_root, data_root, true)
    }

    pub(crate) fn fingerprint<T: Serialize>(value: &T) -> Result<String, BrokerError> {
        let bytes = serde_json::to_vec(value)
            .map_err(|error| BrokerError::json("serialize request fingerprint", error))?;
        Ok(sha256(&bytes))
    }

    pub(crate) fn inspect_existing(
        &self,
        path: &VaultPath,
    ) -> Result<Option<VaultObject>, BrokerError> {
        match self.read_checked(path, Some(writer::disk::CONTENT_LIMIT)) {
            Ok(object) => Ok(Some(object)),
            Err(BrokerError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn snapshot_storage_path(id: &SnapshotId) -> PathBuf {
        Path::new("snapshots").join(format!("{}.json", id.as_str()))
    }

    pub(crate) fn manifest_path(
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<VaultPath, BrokerError> {
        VaultPath::parse(format!(
            "sources/attempts/{task_id}/{attempt_id}/manifest.json"
        ))
        .map_err(|_| BrokerError::InvalidArtifact("derived manifest path was invalid".to_owned()))
    }
}

impl VaultReader for FsVaultBroker {
    fn vault_uuid(&self) -> VaultUuid {
        self.vault_uuid
    }

    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError> {
        let committed = capture::ready(self)?;
        let object = self.read_checked(path, None)?;
        self.check_read_watermark(committed)?;
        Ok(object)
    }
}

impl FsVaultBroker {
    // 仅公开当前读取使用此门禁；安装与证据保全必须仍可私下观察未知对象。
    pub(crate) fn check_read_watermark(&self, committed: u64) -> Result<(), BrokerError> {
        if capture::ready(self)? != committed {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(())
    }

    fn read_checked(
        &self,
        path: &VaultPath,
        max_bytes: Option<usize>,
    ) -> Result<VaultObject, BrokerError> {
        self.writer.check(&self.root, &self.data)?;
        let _read_guard = self.read_guard.lock().map_err(|_| memory_state_error())?;
        self.validate_components(path, false)?;
        let mut file = Self::open_rooted_file(&self.root, path)?;
        let before = file
            .metadata()
            .map_err(|error| BrokerError::io("stat Vault object before read", error))?;
        Self::validate_file_metadata(path, &before)?;
        let mut content = Vec::new();
        if let Some(max) = max_bytes {
            if before.len() > max as u64 {
                return Err(BrokerError::WriterLimitExceeded);
            }
            (&mut file)
                .take(max as u64 + 1)
                .read_to_end(&mut content)
                .map_err(|error| BrokerError::io("read bounded Vault object", error))?;
            if content.len() > max {
                return Err(BrokerError::WriterLimitExceeded);
            }
        } else {
            file.read_to_end(&mut content)
                .map_err(|error| BrokerError::io("read Vault object", error))?;
        }
        let after = file
            .metadata()
            .map_err(|error| BrokerError::io("stat Vault object after read", error))?;
        Self::validate_file_metadata(path, &after)?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err(BrokerError::UnstableRead(path.clone()));
        }
        let current_file = Self::open_rooted_file(&self.root, path)?;
        let current = current_file
            .metadata()
            .map_err(|error| BrokerError::io("stat current Vault object", error))?;
        Self::validate_file_metadata(path, &current)?;
        #[cfg(unix)]
        if before.dev() != current.dev()
            || before.ino() != current.ino()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || after.ctime() != current.ctime()
            || after.ctime_nsec() != current.ctime_nsec()
        {
            return Err(BrokerError::UnstableRead(path.clone()));
        }
        if after.len() != current.len() || after.modified().ok() != current.modified().ok() {
            return Err(BrokerError::UnstableRead(path.clone()));
        }
        self.writer.check(&self.root, &self.data)?;
        Ok(VaultObject {
            revision: Revision::compute(self.vault_uuid, path, &content),
            path: path.clone(),
            content,
        })
    }
}

impl VaultSnapshotter for FsVaultBroker {
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError> {
        // 空输入也必须先过门禁，不能在未知安装期间发布一个成功快照。
        let committed = capture::ready(self)?;
        let mut inputs = inputs.to_vec();
        inputs.sort_by(|left, right| left.path.cmp(&right.path));
        if inputs.windows(2).any(|pair| pair[0].path == pair[1].path) {
            return Err(BrokerError::InvalidArtifact(
                "snapshot contains duplicate paths".to_owned(),
            ));
        }
        let mut files = Vec::with_capacity(inputs.len());
        for input in inputs {
            let object = self.read(&input.path)?;
            if object.revision != input.expected_revision {
                return Err(BrokerError::RevisionConflict {
                    path: input.path,
                    expected: input.expected_revision.to_string(),
                    actual: Some(object.revision),
                });
            }
            files.push(SnapshotFile {
                path: object.path,
                revision: object.revision,
                content: object.content,
            });
        }
        for captured in &files {
            if self.read(&captured.path)?.revision != captured.revision {
                return Err(BrokerError::UnstableRead(captured.path.clone()));
            }
        }
        let id = snapshot_identity(self.vault_uuid, &files)?;
        let snapshot = Snapshot {
            id: id.clone(),
            vault_uuid: self.vault_uuid,
            files,
        };
        let bytes = serde_json::to_vec(&snapshot)
            .map_err(|error| BrokerError::json("serialize snapshot", error))?;
        let storage_path = Self::snapshot_storage_path(&id);
        self.check_read_watermark(committed)?;
        match self.data.open(&storage_path) {
            Ok(_) => {
                if self.load_snapshot(&id)? != snapshot {
                    return Err(BrokerError::CorruptSnapshot);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Self::atomic_write_dir_checked(&self.data, &storage_path, &bytes, true, || {
                    self.check_read_watermark(committed)
                })?
            }
            Err(error) => return Err(BrokerError::io("open snapshot record", error)),
        }
        self.check_read_watermark(committed)?;
        Ok(snapshot)
    }
}

impl SnapshotReader for FsVaultBroker {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError> {
        let mut file = self
            .data
            .open(Self::snapshot_storage_path(id))
            .map_err(|error| BrokerError::io("open snapshot", error))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| BrokerError::io("read snapshot", error))?;
        let snapshot: Snapshot = serde_json::from_slice(&bytes)
            .map_err(|error| BrokerError::json("parse snapshot", error))?;
        if snapshot.id != *id
            || snapshot.vault_uuid != self.vault_uuid
            || snapshot
                .files
                .windows(2)
                .any(|pair| pair[0].path >= pair[1].path)
            || snapshot.files.iter().any(|file| {
                Revision::compute(self.vault_uuid, &file.path, &file.content) != file.revision
            })
            || snapshot_identity(self.vault_uuid, &snapshot.files)? != *id
        {
            return Err(BrokerError::CorruptSnapshot);
        }
        Ok(snapshot)
    }
}
