//! 自有 stage 与实际旧内容的落盘；本模块不凭文件现状恢复 Installing。

use super::record::{Operation, Phase, Stage, cas_matches, conflict};
use crate::writer::{Identity, directory, disk};
use crate::{BrokerError, FsVaultBroker, Mutation, Precondition, VaultObject, sha256};
use cap_std::fs::OpenOptions;
use std::{io::Write, path::Path};

pub(crate) fn history(broker: &FsVaultBroker, object: &VaultObject) -> Result<(), BrokerError> {
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

pub(crate) fn prepare(
    broker: &FsVaultBroker,
    mutation: &Mutation,
    sequence: u64,
) -> Result<Stage, BrokerError> {
    broker.validate_components(&mutation.path, true)?;
    broker.ensure_parent_dirs(&mutation.path)?;
    broker.writer.check(&broker.root, &broker.data)?;
    let dir = directory(&broker.root, ".workbench/staging")?;
    crate::blob::quota::reserve(broker, mutation.content.len() as u64)?;
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

pub(crate) fn verify_stage(broker: &FsVaultBroker, op: &Operation) -> Result<(), BrokerError> {
    stage_proof(broker, op).map(|_| ())
}

fn stage_proof(
    broker: &FsVaultBroker,
    op: &Operation,
) -> Result<crate::blob::file::ContentProof, BrokerError> {
    let dir = directory(&broker.root, ".workbench/staging")?;
    let proof =
        crate::blob::store::verify(broker, &dir, &op.stage.name, &op.path, op.target_revision)?;
    if proof.node.file.identity != op.stage.identity
        || proof.identity.bytes != op.stage.size
        || proof.identity.sha256 != op.stage.digest
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(proof)
}

pub(crate) fn discard_prepared(broker: &FsVaultBroker, op: &Operation) -> Result<(), BrokerError> {
    if op.phase != Phase::Prepared {
        return Err(BrokerError::MutationUnresolved);
    }
    broker.writer.check(&broker.root, &broker.data)?;
    let dir = directory(&broker.root, ".workbench/staging")?;
    if crate::blob::store::exists(&dir, &op.stage.name)? {
        verify_stage(broker, op)?;
        dir.remove_file(&op.stage.name)
            .map_err(|e| BrokerError::io("remove cancelled mutation stage", e))?;
        FsVaultBroker::sync_directory(&dir, Path::new("."))?;
    }
    broker.writer.check(&broker.root, &broker.data)
}

pub(crate) struct PreparedInstall {
    current: crate::blob::current::Current,
    stage: crate::blob::file::ContentProof,
    history: crate::blob::install::HistoryPins,
}

impl PreparedInstall {
    fn check(&self, broker: &FsVaultBroker, op: &Operation) -> Result<(), BrokerError> {
        crate::blob::current::check(broker, &op.path, &self.current)?;
        let stages = directory(&broker.root, ".workbench/staging")?;
        crate::blob::file::check(
            &stages,
            &crate::blob::store::logical(&op.stage.name)?,
            &self.stage.node,
        )?;
        self.history.check(broker, op)
    }
}

pub(crate) fn check_before_install(
    broker: &FsVaultBroker,
    op: &Operation,
) -> Result<PreparedInstall, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    broker.validate_components(&op.path, true)?;
    let stage = stage_proof(broker, op)?;
    let current = crate::blob::current::observe(broker, &op.path)?;
    let actual = current.revision();
    if !cas_matches(op.precondition, actual) {
        return Err(conflict(op.path.clone(), op.precondition, actual));
    }
    // Prepared 重试也重新核验已保全正文；返回以后只重验节点，不在 Installing 中再次大读。
    let history = crate::blob::install::HistoryPins::capture(broker, op)?;
    let sealed = PreparedInstall {
        current,
        stage,
        history,
    };
    #[cfg(test)]
    crate::blob::install::before_history_check_test(broker, op, false)?;
    sealed.check(broker, op)?;
    Ok(sealed)
}

pub(crate) fn install(
    broker: &FsVaultBroker,
    op: &Operation,
    sealed: &PreparedInstall,
) -> Result<(), BrokerError> {
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
    sealed.check(broker, op)?;
    if Identity::directory(&directory(&broker.root, parent_name)?)? != parent_identity {
        return Err(BrokerError::RootChanged);
    }
    #[cfg(test)]
    crate::blob::install::before_history_check_test(broker, op, true)?;
    sealed.history.check(broker, op)?;
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
    #[cfg(test)]
    after_rename_test(broker, &target, name)?;
    let installed = crate::blob::store::verify(
        broker,
        &broker.root,
        op.path.as_str(),
        &op.path,
        op.target_revision,
    )?;
    if installed.node.file.identity != op.stage.identity
        || installed.identity.bytes != op.stage.size
        || installed.identity.sha256 != op.stage.digest
        || Identity::directory(&directory(&broker.root, parent_name)?)? != parent_identity
    {
        return Err(BrokerError::UnstableRead(op.path.clone()));
    }
    broker.writer.check(&broker.root, &broker.data)
}

// 仅单元测试的一次性安装后反例；生产构建没有回调、feature 或环境变量入口。
#[cfg(test)]
pub(crate) fn after_rename_test(
    broker: &FsVaultBroker,
    target: &cap_std::fs::Dir,
    name: &std::ffi::OsStr,
) -> Result<(), BrokerError> {
    use std::sync::atomic::Ordering;
    if broker
        .writer
        .fault
        .compare_exchange(17, 0, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        target
            .hard_link(name, target, ".studiolo-t046-after-rename-hardlink")
            .map_err(|e| BrokerError::io("inject post-install hardlink", e))?;
    }
    Ok(())
}
