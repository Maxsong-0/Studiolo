//! 仅清理已审阅 basename：全目录名称由 scan 先验，不扩大通用文件读取上限。
//! 不接受绝对路径/子路径/回调，根只能来自固定 history 或 snapshots 的封闭 Plan。
use super::scan;
use crate::blob::{
    OBJECT_LIMIT, digest,
    file::{ContentProof, NodeProof, Stamp},
};
use crate::{BrokerError, FsVaultBroker, VaultPath};
use cap_std::fs::{Dir, File, MetadataExt as _};
use rustix::fs::{Mode, OFlags, openat};

fn open(root: &Dir, path: &VaultPath) -> Result<(File, NodeProof), BrokerError> {
    if !scan::owned(path.as_str())? {
        return Err(BrokerError::CorruptWriterState);
    }
    let parent = Stamp::new(
        &root
            .dir_metadata()
            .map_err(|e| BrokerError::io("stat cleanup parent", e))?,
    );
    // 只允许已审阅的单组件 UUID 名，NOFOLLOW/NONBLOCK 拒绝链接或特殊节点。
    let descriptor = openat(
        root,
        path.as_str(),
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| BrokerError::UnsafeNode {
        path: path.clone(),
        reason: "no-link cleanup open failed",
    })?;
    let file = File::from_std(std::fs::File::from(descriptor));
    let metadata = file
        .metadata()
        .map_err(|e| BrokerError::io("stat cleanup object", e))?;
    FsVaultBroker::validate_file_metadata(path, &metadata)?;
    if metadata.nlink() != 1 || metadata.dev() != parent.identity.dev {
        return Err(BrokerError::UnsafeNode {
            path: path.clone(),
            reason: "cleanup object is not local and unique",
        });
    }
    let named = root
        .symlink_metadata(path.as_str())
        .map_err(|e| BrokerError::io("recheck cleanup name", e))?;
    FsVaultBroker::validate_file_metadata(path, &named)?;
    if Stamp::new(&named) != Stamp::new(&metadata)
        || Stamp::new(
            &root
                .dir_metadata()
                .map_err(|e| BrokerError::io("restat cleanup parent", e))?,
        ) != parent
    {
        return Err(BrokerError::CaptureChanged);
    }
    Ok((
        file,
        NodeProof {
            parents: vec![(String::new(), parent)],
            file: Stamp::new(&metadata),
        },
    ))
}

pub(super) fn check(
    root: &Dir,
    path: &VaultPath,
    expected: &NodeProof,
    full_parent: bool,
) -> Result<(), BrokerError> {
    let (_, actual) = open(root, path)?;
    if actual.file != expected.file
        || actual.parents.len() != expected.parents.len()
        || actual
            .parents
            .iter()
            .zip(&expected.parents)
            .any(|(a, b)| a.0 != b.0 || a.1.identity != b.1.identity || (full_parent && a.1 != b.1))
    {
        return Err(BrokerError::CaptureChanged);
    }
    Ok(())
}

pub(super) fn scan(
    root: &Dir,
    path: &VaultPath,
    expected: &Stamp,
) -> Result<ContentProof, BrokerError> {
    let (mut file, node) = open(root, path)?;
    if &node.file != expected {
        return Err(BrokerError::CaptureChanged);
    }
    if expected.bytes > OBJECT_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    // 只读取已计入本批预算的字节，再多读最多一个字节检测增长；不消耗别的候选额度。
    let result = digest::transfer(&mut file, None, expected.bytes, None)?;
    let after = file
        .metadata()
        .map_err(|e| BrokerError::io("restat cleanup object", e))?;
    FsVaultBroker::validate_file_metadata(path, &after)?;
    if Stamp::new(&after) != node.file || result.identity.bytes != node.file.bytes {
        return Err(BrokerError::CaptureChanged);
    }
    check(root, path, &node, true)?;
    Ok(ContentProof {
        node,
        identity: result.identity,
        revision: None,
    })
}
