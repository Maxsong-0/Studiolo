//! 文件和每级父目录均使用 capability 句柄；摘要验证前后拒绝换名与同内容 ABA。
pub(crate) use super::paths::{missing, parents, path_budget};
use super::{BlobIdentity, digest};
use crate::{
    BrokerError, FsVaultBroker, Revision, VaultPath, VaultUuid,
    writer::{Identity, directory},
};
use cap_std::fs::{Dir, File, Metadata, MetadataExt as _};
use rustix::fs::{Mode, OFlags, openat};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Stamp {
    pub identity: Identity,
    pub bytes: u64,
    pub ctime: i64,
    pub ctime_nanos: i64,
    pub mtime: i64,
    pub mtime_nanos: i64,
}
impl Stamp {
    pub(crate) fn new(value: &Metadata) -> Self {
        Self {
            identity: Identity::file(value),
            bytes: value.len(),
            ctime: value.ctime(),
            ctime_nanos: value.ctime_nsec(),
            mtime: value.mtime(),
            mtime_nanos: value.mtime_nsec(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NodeProof {
    pub parents: Vec<(String, Stamp)>,
    pub file: Stamp,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContentProof {
    pub node: NodeProof,
    pub identity: BlobIdentity,
    pub revision: Option<Revision>,
}
pub(crate) struct Opened {
    file: File,
    node: NodeProof,
}

pub(crate) fn open(root: &Dir, path: &VaultPath) -> Result<Opened, BrokerError> {
    let before = parents(root, path)?;
    let (prefix, _) = before.last().ok_or(BrokerError::CorruptWriterState)?;
    let parent = directory(root, if prefix.is_empty() { "." } else { prefix })?;
    let name = path
        .as_str()
        .rsplit('/')
        .next()
        .ok_or(BrokerError::CorruptWriterState)?;
    let handle = openat(
        &parent,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|e| {
        if e == rustix::io::Errno::NOENT {
            BrokerError::NotFound(path.clone())
        } else {
            BrokerError::UnsafeNode {
                path: path.clone(),
                reason: "no-link blob open failed",
            }
        }
    })?;
    let file = File::from_std(std::fs::File::from(handle));
    let metadata = file
        .metadata()
        .map_err(|e| BrokerError::io("stat blob", e))?;
    FsVaultBroker::validate_file_metadata(path, &metadata)?;
    if metadata.nlink() != 1 || metadata.dev() != Identity::directory(root)?.dev {
        return Err(BrokerError::UnsafeNode {
            path: path.clone(),
            reason: "blob identity is not local and unique",
        });
    }
    if parents(root, path)? != before {
        return Err(BrokerError::UnstableRead(path.clone()));
    }
    Ok(Opened {
        file,
        node: NodeProof {
            parents: before,
            file: Stamp::new(&metadata),
        },
    })
}
pub(crate) fn check(root: &Dir, path: &VaultPath, expected: &NodeProof) -> Result<(), BrokerError> {
    if open(root, path)?.node != *expected {
        Err(BrokerError::UnstableRead(path.clone()))
    } else {
        Ok(())
    }
}
pub(crate) fn scan(
    root: &Dir,
    path: &VaultPath,
    limit: u64,
    binding: Option<(VaultUuid, &VaultPath)>,
) -> Result<ContentProof, BrokerError> {
    transfer(root, path, None, limit, binding)
}
pub(crate) fn transfer(
    root: &Dir,
    path: &VaultPath,
    output: Option<&mut File>,
    limit: u64,
    binding: Option<(VaultUuid, &VaultPath)>,
) -> Result<ContentProof, BrokerError> {
    let mut opened = open(root, path)?;
    if opened.node.file.bytes > limit {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let result = digest::transfer(&mut opened.file, output, limit, binding)?;
    let after = opened
        .file
        .metadata()
        .map_err(|e| BrokerError::io("restat blob source", e))?;
    FsVaultBroker::validate_file_metadata(path, &after)?;
    if Stamp::new(&after) != opened.node.file || result.identity.bytes != opened.node.file.bytes {
        return Err(BrokerError::UnstableRead(path.clone()));
    }
    check(root, path, &opened.node)?;
    Ok(ContentProof {
        node: opened.node,
        identity: result.identity,
        revision: result.revision,
    })
}

// 批量删除会改变同一目录的 ctime；仍重验所有父目录身份和文件完整元数据。
pub(crate) fn check_file(
    root: &Dir,
    path: &VaultPath,
    expected: &NodeProof,
) -> Result<(), BrokerError> {
    let actual = open(root, path)?.node;
    if actual.file != expected.file
        || actual.parents.len() != expected.parents.len()
        || actual
            .parents
            .iter()
            .zip(&expected.parents)
            .any(|(a, b)| a.0 != b.0 || a.1.identity != b.1.identity)
    {
        return Err(BrokerError::CaptureChanged);
    }
    Ok(())
}

pub(crate) fn observe_node(root: &Dir, path: &VaultPath) -> Result<NodeProof, BrokerError> {
    open(root, path).map(|opened| opened.node)
}
