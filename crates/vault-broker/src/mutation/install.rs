//! 自有 stage 与实际旧内容的落盘；本模块不凭文件现状恢复 Installing。

use super::record::{Operation, Phase, Stage, cas_matches, conflict};
use crate::writer::{Identity, directory, disk};
use crate::{BrokerError, FsVaultBroker, Mutation, Precondition, Revision, VaultObject, sha256};
use cap_std::fs::OpenOptions;
use std::{io::Write, path::Path};

pub(super) fn history(broker: &FsVaultBroker, object: &VaultObject) -> Result<(), BrokerError> {
    if object.content.len() > disk::CONTENT_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    broker.writer.check(&broker.root, &broker.data)?;
    let dir = directory(&broker.data, "history")?;
    let name = object.revision.to_string();
    match disk::read(&dir, &name, disk::CONTENT_LIMIT)? {
        Some(bytes) if bytes != object.content => return Err(BrokerError::CorruptWriterState),
        Some(_) => {}
        None => FsVaultBroker::atomic_write_dir_checked(
            &dir,
            Path::new(&name),
            &object.content,
            true,
            || broker.writer.check(&broker.root, &broker.data),
        )?,
    }
    // 旧格式已有对象也同步；历史权威是实际读取的旧 revision 和原字节。
    let logical = crate::VaultPath::parse(&name).map_err(|_| BrokerError::CorruptWriterState)?;
    FsVaultBroker::open_rooted_file(&dir, &logical)?
        .sync_all()
        .map_err(|e| BrokerError::io("sync preserved history", e))?;
    FsVaultBroker::sync_directory(&dir, Path::new("."))?;
    if disk::read(&dir, &name, disk::CONTENT_LIMIT)?.as_deref() != Some(object.content.as_slice()) {
        return Err(BrokerError::CorruptWriterState);
    }
    broker.writer.check(&broker.root, &broker.data)
}

pub(super) fn prepare(
    broker: &FsVaultBroker,
    mutation: &Mutation,
    sequence: u64,
) -> Result<Stage, BrokerError> {
    broker.validate_components(&mutation.path, true)?;
    broker.ensure_parent_dirs(&mutation.path)?;
    broker.writer.check(&broker.root, &broker.data)?;
    let dir = directory(&broker.root, ".workbench/staging")?;
    // 崩溃于 Prepared 之前可以留下孤立 stage，但不能无限增加目录项。
    for (index, entry) in dir
        .entries()
        .map_err(|e| BrokerError::io("enumerate mutation stages", e))?
        .enumerate()
    {
        if index >= 63 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect mutation stage", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        if !disk::hex(name.strip_suffix(".stage").unwrap_or("")) {
            return Err(BrokerError::CorruptWriterState);
        }
        let path = crate::VaultPath::parse(&name).map_err(|_| BrokerError::CorruptWriterState)?;
        let metadata = entry
            .metadata()
            .map_err(|e| BrokerError::io("stat retained mutation stage", e))?;
        FsVaultBroker::validate_file_metadata(&path, &metadata)?;
        if metadata.len() > disk::CONTENT_LIMIT as u64 {
            return Err(BrokerError::WriterLimitExceeded);
        }
    }
    let nonce = format!(
        "{}:{sequence}:{}",
        broker.writer.binding.pair_id,
        crate::VaultUuid::new()
    );
    let name = format!("{}.stage", sha256(nonce.as_bytes()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = dir
        .open_with(&name, &options)
        .map_err(|e| BrokerError::io("create mutation stage", e))?;
    file.write_all(&mutation.content)
        .map_err(|e| BrokerError::io("write mutation stage", e))?;
    file.sync_all()
        .map_err(|e| BrokerError::io("sync mutation stage", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| BrokerError::io("stat mutation stage", e))?;
    let logical = crate::VaultPath::parse(&name).map_err(|_| BrokerError::CorruptWriterState)?;
    FsVaultBroker::validate_file_metadata(&logical, &metadata)?;
    FsVaultBroker::sync_directory(&dir, Path::new("."))?;
    let stage = Stage {
        name,
        identity: Identity::file(&metadata),
        digest: sha256(&mutation.content),
        size: mutation.content.len() as u64,
    };
    broker.writer.check(&broker.root, &broker.data)?;
    Ok(stage)
}

pub(super) fn verify_stage(broker: &FsVaultBroker, op: &Operation) -> Result<(), BrokerError> {
    let dir = directory(&broker.root, ".workbench/staging")?;
    let (bytes, identity) = disk::read_identified(&dir, &op.stage.name, disk::CONTENT_LIMIT)?
        .ok_or(BrokerError::CorruptWriterState)?;
    if identity != op.stage.identity
        || bytes.len() as u64 != op.stage.size
        || sha256(&bytes) != op.stage.digest
        || Revision::compute(broker.vault_uuid, &op.path, &bytes) != op.target_revision
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(())
}

pub(super) fn discard_prepared(broker: &FsVaultBroker, op: &Operation) -> Result<(), BrokerError> {
    if op.phase != Phase::Prepared {
        return Err(BrokerError::MutationUnresolved);
    }
    broker.writer.check(&broker.root, &broker.data)?;
    let dir = directory(&broker.root, ".workbench/staging")?;
    if disk::read(&dir, &op.stage.name, disk::CONTENT_LIMIT)?.is_some() {
        verify_stage(broker, op)?;
        dir.remove_file(&op.stage.name)
            .map_err(|e| BrokerError::io("remove cancelled mutation stage", e))?;
        FsVaultBroker::sync_directory(&dir, Path::new("."))?;
    }
    broker.writer.check(&broker.root, &broker.data)
}

pub(super) fn check_before_install(
    broker: &FsVaultBroker,
    op: &Operation,
) -> Result<(), BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    broker.validate_components(&op.path, true)?;
    verify_stage(broker, op)?;
    let actual = match broker.read_checked(&op.path, Some(disk::CONTENT_LIMIT)) {
        Ok(object) => Some(object.revision),
        Err(BrokerError::NotFound(_)) => None,
        Err(e) => return Err(e),
    };
    if !cas_matches(op.precondition, actual) {
        return Err(conflict(op.path.clone(), op.precondition, actual));
    }
    Ok(())
}

pub(super) fn install(broker: &FsVaultBroker, op: &Operation) -> Result<(), BrokerError> {
    if op.phase != Phase::Installing {
        return Err(BrokerError::CorruptWriterState);
    }
    let path = Path::new(op.path.as_str());
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent_name = parent.to_str().ok_or(BrokerError::CorruptWriterState)?;
    let target = directory(&broker.root, parent_name)?;
    let parent_identity = Identity::directory(&target)?;
    let stages = directory(&broker.root, ".workbench/staging")?;
    let name = path.file_name().ok_or(BrokerError::CorruptWriterState)?;
    check_before_install(broker, op)?;
    if Identity::directory(&directory(&broker.root, parent_name)?)? != parent_identity {
        return Err(BrokerError::RootChanged);
    }
    match op.precondition {
        Precondition::MustNotExist => {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            rustix::fs::renameat_with(
                &stages,
                &op.stage.name,
                &target,
                name,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(|e| BrokerError::io("install create-only mutation", e.into()))?;
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                stages
                    .hard_link(&op.stage.name, &target, name)
                    .map_err(|e| BrokerError::io("install create-only mutation", e))?;
                stages
                    .remove_file(&op.stage.name)
                    .map_err(|e| BrokerError::io("unlink mutation stage", e))?;
            }
        }
        Precondition::Match(_) => stages
            .rename(&op.stage.name, &target, name)
            .map_err(|e| BrokerError::io("install replacement mutation", e))?,
    }
    FsVaultBroker::sync_directory(&stages, Path::new("."))?;
    FsVaultBroker::sync_directory(&target, Path::new("."))?;
    let installed = broker.read_checked(&op.path, Some(disk::CONTENT_LIMIT))?;
    let (_, identity) = disk::read_identified(&broker.root, op.path.as_str(), disk::CONTENT_LIMIT)?
        .ok_or(BrokerError::CorruptWriterState)?;
    if installed.revision != op.target_revision
        || identity != op.stage.identity
        || Identity::directory(&directory(&broker.root, parent_name)?)? != parent_identity
    {
        return Err(BrokerError::UnstableRead(op.path.clone()));
    }
    broker.writer.check(&broker.root, &broker.data)
}
