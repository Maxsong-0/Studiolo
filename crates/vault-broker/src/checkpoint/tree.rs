//! 目录与文件都有固定预算；封存摘要覆盖名字、目录结构、空目录及全部文件字节。
//! 唯一跳过的是开库清扫的隔离区（`crate::residue`）：它不在配对布局里，里面的残留备份也归不了类。
use crate::{
    BrokerError, FsVaultBroker,
    writer::{Identity, directory},
};
use cap_std::fs::{Dir, MetadataExt as _};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Instant};
const MAX_NODES: u64 = 1_000_000;
const MAX_BYTES: u64 = 512 * 1024 * 1024 * 1024;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TreeSeal {
    hash: [u8; 32],
    nodes: u64,
    bytes: u64,
}
pub(super) fn check_pair(left: &TreeSeal, right: &TreeSeal) -> Result<(), BrokerError> {
    if left
        .nodes
        .checked_add(right.nodes)
        .is_none_or(|v| v > MAX_NODES)
        || left
            .bytes
            .checked_add(right.bytes)
            .is_none_or(|v| v > MAX_BYTES)
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(())
}
/// `skip` 是要跳过的隔离区，从根起逐段写（`residue::VAULT_QUARANTINE`／`DATA_QUARANTINE`）。
pub(super) fn scan(root: &Dir, skip: &[&str], deadline: Instant) -> Result<TreeSeal, BrokerError> {
    run(root, None, skip, deadline)
}
pub(super) fn copy(
    root: &Dir,
    target: &Dir,
    skip: &[&str],
    deadline: Instant,
) -> Result<TreeSeal, BrokerError> {
    run(root, Some(target), skip, deadline)
}
fn run(
    root: &Dir,
    target: Option<&Dir>,
    skip: &[&str],
    deadline: Instant,
) -> Result<TreeSeal, BrokerError> {
    let mut walk = Walk {
        device: Identity::directory(root)?.dev,
        deadline,
        nodes: 0,
        bytes: 0,
        buffer: vec![0; 256 * 1024],
    };
    let hash = walk.dir(root, target, skip, 0)?;
    Ok(TreeSeal {
        hash,
        nodes: walk.nodes,
        bytes: walk.bytes,
    })
}
struct Walk {
    device: u64,
    deadline: Instant,
    nodes: u64,
    bytes: u64,
    buffer: Vec<u8>,
}
impl Walk {
    fn dir(
        &mut self,
        source: &Dir,
        target: Option<&Dir>,
        skip: &[&str],
        depth: usize,
    ) -> Result<[u8; 32], BrokerError> {
        super::check_deadline(self.deadline)?;
        if depth > MAX_DEPTH {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let before = source
            .dir_metadata()
            .map_err(|e| BrokerError::io("核对封存源目录", e))?;
        if !before.is_dir()
            || before.dev() != self.device
            || before.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(BrokerError::IncompleteCapture);
        }
        let names = listed(source, skip, MAX_NODES - self.nodes, self.deadline)?;
        self.nodes += names.len() as u64;
        let mut hash = Sha256::new();
        hash.update(b"studiolo-broker-tree-v1\0");
        hash.update((names.len() as u64).to_be_bytes());
        for name in &names {
            super::check_deadline(self.deadline)?;
            let meta = source
                .symlink_metadata(name)
                .map_err(|e| BrokerError::io("核对封存节点", e))?;
            hash.update((name.len() as u64).to_be_bytes());
            hash.update(name.as_bytes());
            if meta.is_dir() && !meta.file_type().is_symlink() {
                let child = directory(source, name)?;
                if Identity::directory(&child)? != Identity::file(&meta) {
                    return Err(BrokerError::CaptureChanged);
                }
                let copied = target.map(|dir| super::copy::dir(dir, name)).transpose()?;
                hash.update([1]);
                hash.update(self.dir(&child, copied.as_ref(), within(skip, name), depth + 1)?);
                if Identity::directory(&directory(source, name)?)? != Identity::file(&meta) {
                    return Err(BrokerError::CaptureChanged);
                }
            } else if meta.is_file() && !meta.file_type().is_symlink() {
                let (digest, length) = super::files::stream(
                    source,
                    name,
                    target,
                    self.device,
                    MAX_BYTES - self.bytes,
                    self.deadline,
                    &mut self.buffer,
                )?;
                self.bytes += length;
                hash.update([2]);
                hash.update(length.to_be_bytes());
                hash.update(digest);
            } else {
                return Err(BrokerError::IncompleteCapture);
            }
        }
        let after = source
            .dir_metadata()
            .map_err(|e| BrokerError::io("复核封存源目录", e))?;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.mode() != after.mode()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || names != listed(source, skip, MAX_NODES, self.deadline)?
        {
            return Err(BrokerError::CaptureChanged);
        }
        if let Some(target) = target {
            FsVaultBroker::sync_directory(target, Path::new("."))?;
        }
        super::check_deadline(self.deadline)?;
        Ok(hash.finalize().into())
    }
}
/// 往下一层时还要跳过的路径：只有正走在跳过路径上才往下传。
fn within<'a>(skip: &'a [&'a str], name: &str) -> &'a [&'a str] {
    match skip {
        [first, rest @ ..] if *first == name && !rest.is_empty() => rest,
        _ => &[],
    }
}
/// 一层目录的名字，去掉跳过路径落在这一层的那个隔离区。只跳过真目录：
/// 同名的文件照常进封存，同名的链接照旧让封存失败。
fn listed(
    dir: &Dir,
    skip: &[&str],
    remaining: u64,
    deadline: Instant,
) -> Result<Vec<String>, BrokerError> {
    let mut names = names(dir, remaining, deadline)?;
    if let [last] = skip
        && dir
            .symlink_metadata(last)
            .is_ok_and(|metadata| metadata.is_dir())
    {
        names.retain(|name| name != last);
    }
    Ok(names)
}
fn names(dir: &Dir, remaining: u64, deadline: Instant) -> Result<Vec<String>, BrokerError> {
    let mut names = Vec::new();
    for entry in dir
        .entries()
        .map_err(|e| BrokerError::io("枚举封存目录", e))?
    {
        super::check_deadline(deadline)?;
        if names.len() as u64 >= remaining {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("读取封存目录项", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::IncompleteCapture)?;
        if name.is_empty() || name.len() > 255 || name.chars().any(char::is_control) {
            return Err(BrokerError::IncompleteCapture);
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}
