//! 仅供同 crate 完整名字枚举后消费封闭 NodeProof；不暴露句柄或任意读取回调。
use super::{
    digest,
    file::{ContentProof, NodeProof, Stamp},
};
use crate::{
    BrokerError, FsVaultBroker, VaultPath, VaultUuid,
    writer::{Identity, directory},
};
use cap_std::fs::{Dir, File, MetadataExt as _};
use rustix::fs::{Mode, OFlags, openat};

pub(crate) fn directory_stamp(root: &Dir, path: &str, private: bool) -> Result<Stamp, BrokerError> {
    let dir = directory(root, if path.is_empty() { "." } else { path })?;
    let stamp = opened_directory_stamp(&dir, private)?;
    if stamp.identity.dev != Identity::directory(root)?.dev {
        return Err(BrokerError::WriterBindingMismatch);
    }
    Ok(stamp)
}
pub(crate) fn opened_directory_stamp(dir: &Dir, private: bool) -> Result<Stamp, BrokerError> {
    crate::readonly::sync::check_directory(dir, private)?;
    dir.dir_metadata()
        .map(|metadata| Stamp::new(&metadata))
        .map_err(|e| BrokerError::io("stat observed directory", e))
}
pub(crate) fn parents_check(
    root: &Dir,
    parents: &[(String, Stamp)],
    private: bool,
) -> Result<(), BrokerError> {
    for (path, stamp) in parents {
        if directory_stamp(root, path, private)? != *stamp {
            return Err(BrokerError::CaptureChanged);
        }
    }
    Ok(())
}
fn open(
    root: &Dir,
    path: &VaultPath,
    node: &NodeProof,
    private: bool,
) -> Result<File, BrokerError> {
    parents_check(root, &node.parents, private)?;
    let (prefix, expected) = node.parents.last().ok_or(BrokerError::CaptureChanged)?;
    let parent = directory(root, if prefix.is_empty() { "." } else { prefix })?;
    if opened_directory_stamp(&parent, private)? != *expected {
        return Err(BrokerError::CaptureChanged);
    }
    let name = path
        .as_str()
        .rsplit('/')
        .next()
        .ok_or(BrokerError::CaptureChanged)?;
    let fd = openat(
        &parent,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| BrokerError::CaptureChanged)?;
    let file = File::from_std(std::fs::File::from(fd));
    let metadata = file
        .metadata()
        .map_err(|e| BrokerError::io("stat observed file", e))?;
    FsVaultBroker::validate_file_metadata(path, &metadata)?;
    crate::readonly::sync::check_file(&metadata, private)?;
    if metadata.nlink() != 1
        || Stamp::new(&metadata) != node.file
        || metadata.dev() != Identity::directory(root)?.dev
    {
        return Err(BrokerError::CaptureChanged);
    }
    parents_check(root, &node.parents, private)?;
    Ok(file)
}
pub(crate) fn check(
    root: &Dir,
    path: &VaultPath,
    node: &NodeProof,
    private: bool,
) -> Result<(), BrokerError> {
    open(root, path, node, private).map(|_| ())
}
pub(crate) fn facts(
    root: &Dir,
    path: &VaultPath,
    node: &NodeProof,
    limit: u64,
    vault: Option<VaultUuid>,
    private: bool,
) -> Result<(ContentProof, bool), BrokerError> {
    let mut file = open(root, path, node, private)?;
    if node.file.bytes > limit {
        return Err(BrokerError::WriterLimitExceeded);
    }
    // 正文实际读取严格止于已验证长度，增长由最终身份复核拒绝，不额外读取探测字节。
    let (facts, utf8) = digest::facts(&mut file, node.file.bytes, vault.map(|v| (v, path)))?;
    let after = file
        .metadata()
        .map_err(|e| BrokerError::io("restat observed file", e))?;
    crate::readonly::sync::check_file(&after, private)?;
    if Stamp::new(&after) != node.file || facts.identity.bytes != node.file.bytes {
        return Err(BrokerError::CaptureChanged);
    }
    check(root, path, node, private)?;
    Ok((
        ContentProof {
            node: node.clone(),
            identity: facts.identity,
            revision: facts.revision,
        },
        utf8,
    ))
}
pub(crate) fn missing(
    root: &Dir,
    path: &VaultPath,
    parents: &[(String, Stamp)],
    private: bool,
) -> Result<(), BrokerError> {
    parents_check(root, parents, private)?;
    let (prefix, expected) = parents.last().ok_or(BrokerError::CaptureChanged)?;
    let parent = directory(root, if prefix.is_empty() { "." } else { prefix })?;
    if opened_directory_stamp(&parent, private)? != *expected {
        return Err(BrokerError::CaptureChanged);
    }
    let suffix = if prefix.is_empty() {
        path.as_str()
    } else {
        path.as_str()
            .strip_prefix(prefix)
            .and_then(|s| s.strip_prefix('/'))
            .ok_or(BrokerError::CaptureChanged)?
    };
    let component = suffix
        .split('/')
        .next()
        .ok_or(BrokerError::CaptureChanged)?;
    match parent.symlink_metadata(component) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err(BrokerError::CaptureChanged),
    }
    parents_check(root, parents, private)
}
