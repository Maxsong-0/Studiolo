//! 大历史/证据只在完整复制并同步后以 create-only 发布；已有对象必须逐字节摘要一致。
use super::{BlobIdentity, OBJECT_LIMIT, file};
use crate::{BrokerError, FsVaultBroker, Revision, VaultPath, writer::Identity};
use cap_std::fs::{Dir, OpenOptions};
use std::path::Path;

pub(crate) fn logical(value: &str) -> Result<VaultPath, BrokerError> {
    VaultPath::parse(value).map_err(|_| BrokerError::CorruptWriterState)
}
pub(crate) fn verify(
    broker: &FsVaultBroker,
    root: &Dir,
    name: &str,
    path: &VaultPath,
    revision: Revision,
) -> Result<file::ContentProof, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    let proof = file::scan(
        root,
        &logical(name)?,
        OBJECT_LIMIT,
        Some((broker.vault_uuid, path)),
    )?;
    if proof.revision != Some(revision) {
        return Err(BrokerError::CorruptWriterState);
    }
    broker.writer.check(&broker.root, &broker.data)?;
    Ok(proof)
}
pub(crate) fn copy(
    broker: &FsVaultBroker,
    input: &Dir,
    source: &VaultPath,
    output: &Dir,
    name: &str,
    binding: (&VaultPath, Revision),
    expected: &BlobIdentity,
) -> Result<file::ContentProof, BrokerError> {
    let (path, revision) = binding;
    expected.validate()?;
    broker.writer.check(&broker.root, &broker.data)?;
    match verify(broker, output, name, path, revision) {
        Ok(existing) => {
            if existing.identity != *expected {
                return Err(BrokerError::CorruptWriterState);
            }
            FsVaultBroker::open_rooted_file(output, &logical(name)?)?
                .sync_all()
                .map_err(|e| BrokerError::io("sync existing preserved blob", e))?;
            FsVaultBroker::sync_directory(output, Path::new("."))?;
            file::check(output, &logical(name)?, &existing.node)?;
            broker.writer.check(&broker.root, &broker.data)?;
            return Ok(existing);
        }
        Err(BrokerError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    // 临时文件不是历史索引或 canonical；失败只留下受控、未发布的临时字节。
    super::quota::reserve_preservation(output, expected.bytes)?;
    let temp_name = format!(".studiolo-blob-{}.tmp", crate::VaultUuid::new());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut temp = output
        .open_with(&temp_name, &options)
        .map_err(|e| BrokerError::io("create preserved blob", e))?;
    let created = Identity::file(
        &temp
            .metadata()
            .map_err(|e| BrokerError::io("stat new preserved blob", e))?,
    );
    let result = (|| {
        let copied = file::transfer(
            input,
            source,
            Some(&mut temp),
            expected.bytes,
            Some((broker.vault_uuid, path)),
        )?;
        if copied.identity != *expected || copied.revision != Some(revision) {
            return Err(BrokerError::CorruptWriterState);
        }
        temp.sync_all()
            .map_err(|e| BrokerError::io("sync preserved blob", e))?;
        let identity = Identity::file(
            &temp
                .metadata()
                .map_err(|e| BrokerError::io("stat preserved blob", e))?,
        );
        broker.writer.check(&broker.root, &broker.data)?;
        let sealed = verify(broker, output, &temp_name, path, revision)?;
        if sealed.identity != *expected || sealed.node.file.identity != identity {
            return Err(BrokerError::CaptureChanged);
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        rustix::fs::renameat_with(
            output,
            &temp_name,
            output,
            name,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|e| BrokerError::io("publish preserved blob", e.into()))?;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return Err(BrokerError::WriterLimitExceeded);
        FsVaultBroker::sync_directory(output, Path::new("."))?;
        let published = verify(broker, output, name, path, revision)?;
        if published.identity != *expected || published.node.file.identity != identity {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(published)
    })();
    // 不动未知目标；仅尽力清理本次 create_new 的随机临时名。
    if result.is_err()
        && broker.writer.check(&broker.root, &broker.data).is_ok()
        && output
            .symlink_metadata(&temp_name)
            .is_ok_and(|metadata| Identity::file(&metadata) == created)
    {
        let _ = output.remove_file(&temp_name);
    }
    result
}

pub(crate) fn exists(root: &Dir, name: &str) -> Result<bool, BrokerError> {
    match file::open(root, &logical(name)?) {
        Ok(_) => Ok(true),
        Err(BrokerError::NotFound(_)) => Ok(false),
        Err(error) => Err(error),
    }
}
