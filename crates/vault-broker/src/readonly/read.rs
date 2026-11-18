//! 共享的有界稳定读取；调用方只能选择 revision 命名空间。

use cap_std::fs::{File, Metadata};
use std::io::Read as _;
use studiolo_core::{Revision, VaultPath, VaultUuid};

use super::{ReadNote, rooted};
use crate::BrokerError;

pub(super) fn replica_namespace() -> VaultUuid {
    "d67f1358-fd5f-43ca-82a4-704805a83ed2"
        .parse()
        .expect("the read-only namespace is a valid UUID")
}

pub(super) fn read_with(
    path: &VaultPath,
    max_bytes: u64,
    namespace: VaultUuid,
    mut open: impl FnMut() -> Result<File, BrokerError>,
) -> Result<ReadNote, BrokerError> {
    let file = open()?;
    let before = file
        .metadata()
        .map_err(|error| BrokerError::io("stat read-only file", error))?;
    rooted::regular(path, &before)?;
    if before.len() > max_bytes {
        return Err(exceeded(path, max_bytes));
    }
    finish_read(path, max_bytes, namespace, file, before, open)
}

fn finish_read(
    path: &VaultPath,
    max_bytes: u64,
    namespace: VaultUuid,
    mut file: File,
    before: Metadata,
    mut open: impl FnMut() -> Result<File, BrokerError>,
) -> Result<ReadNote, BrokerError> {
    // 按打开时的长度一次分配到位：从空缓冲倍增读满，平均多占四成多，整读校验时每份正文都带着这些余量
    // （T-215：24 MiB 正文的快照实占 34.7 MiB）。长度变了后面的稳定性检查照样拒绝，这里只是初始容量。
    let mut content = Vec::with_capacity(usize::try_from(before.len().min(max_bytes)).unwrap_or(0));
    (&mut file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut content)
        .map_err(|error| BrokerError::io("read bounded file", error))?;
    if content.len() as u64 > max_bytes {
        return Err(exceeded(path, max_bytes));
    }
    let after = file
        .metadata()
        .map_err(|error| BrokerError::io("stat read-only file", error))?;
    rooted::regular(path, &after)?;
    let current = open()?
        .metadata()
        .map_err(|error| BrokerError::io("recheck read-only file", error))?;
    rooted::regular(path, &current)?;
    if !rooted::unchanged(&before, &after)
        || !rooted::unchanged(&after, &current)
        || before.len() != content.len() as u64
    {
        return Err(BrokerError::UnstableRead(path.clone()));
    }
    Ok(ReadNote {
        revision: Revision::compute(namespace, path, &content),
        path: path.clone(),
        content,
    })
}

fn exceeded(path: &VaultPath, max_bytes: u64) -> BrokerError {
    BrokerError::ReadLimitExceeded {
        path: path.clone(),
        max_bytes,
    }
}

#[cfg(all(test, unix))]
mod tests;
