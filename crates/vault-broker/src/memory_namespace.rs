//! 内存替身的路径命名空间守卫：把磁盘实现里由文件系统天然强制的不变量，在扁平 map 上重建。
//!
//! 替身与真实实现语义不一致会让基于替身的测试给出磁盘拿不到的信心，因此这里逐条照抄
//! `FsVaultBroker` 的判定，包括读写两条路径**不同**的顺序与错误载荷：
//!
//! - 读走 `FsVaultBroker::validate_components`：`PathCollision.existing` 带父前缀；
//!   父链上出现普通文件报 `non-directory path component`，末段是目录报 `not a regular file`。
//! - 写走 `blob::paths::{path_budget, parents, missing}`：先过固定路径预算，
//!   `PathCollision.existing` 只有同级条目名（不带父前缀）；末段是目录同样报
//!   `not a regular file`，而父链上出现普通文件时，磁盘在 `writer::lease::child`
//!   以 `NotADirectory` 失败，替身照抄该错误而不是自造一个更漂亮的结构化变体。
//!
//! 最后一条是有意为之的照抄：若替身在此处返回 `UnsafeNode`，替身与磁盘就又一次
//! 分叉。磁盘侧把它收敛为结构化错误属于 `FsVaultBroker` 的改动，见 T-053 REPORT。

use std::collections::BTreeMap;

use crate::{BrokerError, MemoryVaultBroker, VaultIdentity, VaultPath, VaultUuid};

/// canonical 根上的 Vault 身份 marker。`FsVaultBroker::read` 可以读到它（写入另由
/// `VaultPath::is_protected` 拒绝），替身必须提供同一条可读路径。
pub(crate) const IDENTITY_MARKER: &str = ".workbench/vault.json";

/// 与 `writer::disk::create` 相同的紧凑 JSON 编码，保证替身与磁盘的 marker 字节一致，
/// 因而 `Revision::compute` 出的 revision 也一致。
pub(crate) fn identity_bytes(vault_uuid: VaultUuid) -> Result<Vec<u8>, BrokerError> {
    serde_json::to_vec(&VaultIdentity {
        format_version: 1,
        vault_uuid,
    })
    .map_err(|error| BrokerError::json("serialize memory Vault identity", error))
}

/// `parent` 下的直接子条目名 → 该名是否被某个对象整体占用（true 表示普通文件，false 表示目录）。
fn entries(broker: &MemoryVaultBroker, parent: &str) -> BTreeMap<String, bool> {
    let mut listing = BTreeMap::new();
    let stored = broker
        .objects
        .keys()
        .map(VaultPath::as_str)
        .chain(std::iter::once(IDENTITY_MARKER));
    for path in stored {
        let Some(rest) = remainder(path, parent) else {
            continue;
        };
        let (head, is_file) = match rest.split_once('/') {
            Some((head, _)) => (head, false),
            None => (rest, true),
        };
        listing
            .entry(head.to_owned())
            .and_modify(|existing| *existing |= is_file)
            .or_insert(is_file);
    }
    listing
}

/// `path` 落在 `parent` 目录下时返回其相对部分；`parent` 为空表示 canonical 根。
fn remainder<'a>(path: &'a str, parent: &str) -> Option<&'a str> {
    if parent.is_empty() {
        return Some(path);
    }
    path.strip_prefix(parent)?.strip_prefix('/')
}

/// 同一父目录下折叠同名但拼写不同的条目。
fn aliased<'a>(listing: &'a BTreeMap<String, bool>, component: &str) -> Option<&'a str> {
    let requested = crate::collision_component_key(component);
    listing
        .keys()
        .find(|existing| {
            existing.as_str() != component && crate::collision_component_key(existing) == requested
        })
        .map(String::as_str)
}

/// 读取前的命名空间检查，对齐 `FsVaultBroker::validate_components(path, false)`。
pub(crate) fn guard_read(broker: &MemoryVaultBroker, path: &VaultPath) -> Result<(), BrokerError> {
    let components: Vec<&str> = path.as_str().split('/').collect();
    let mut parent = String::new();
    for (index, component) in components.iter().enumerate() {
        let listing = entries(broker, &parent);
        if let Some(existing) = aliased(&listing, component) {
            return Err(BrokerError::PathCollision {
                requested: path.clone(),
                existing: if parent.is_empty() {
                    existing.to_owned()
                } else {
                    format!("{parent}/{existing}")
                },
            });
        }
        let Some(&is_file) = listing.get(*component) else {
            return Err(BrokerError::NotFound(path.clone()));
        };
        let is_last = index + 1 == components.len();
        if !is_last && is_file {
            return Err(BrokerError::UnsafeNode {
                path: path.clone(),
                reason: "non-directory path component",
            });
        }
        if is_last && !is_file {
            return Err(BrokerError::UnsafeNode {
                path: path.clone(),
                reason: "not a regular file",
            });
        }
        descend(&mut parent, component);
    }
    Ok(())
}

/// 写入前的命名空间检查，对齐 `blob::paths::{path_budget, missing}` 的逐级观察。
pub(crate) fn guard_write(broker: &MemoryVaultBroker, path: &VaultPath) -> Result<(), BrokerError> {
    if path.as_str().len() > 4096 || path.as_str().split('/').count() > 64 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let components: Vec<&str> = path.as_str().split('/').collect();
    let mut parent = String::new();
    for (index, component) in components.iter().enumerate() {
        let listing = entries(broker, &parent);
        if let Some(existing) = aliased(&listing, component) {
            return Err(BrokerError::PathCollision {
                requested: path.clone(),
                existing: existing.to_owned(),
            });
        }
        let is_last = index + 1 == components.len();
        match listing.get(*component) {
            // 本级起余下路径都不存在：磁盘在这里停止观察并逐级创建目录。
            None => return Ok(()),
            Some(&true) if !is_last => {
                return Err(BrokerError::io(
                    "open writer layout",
                    std::io::Error::from(std::io::ErrorKind::NotADirectory),
                ));
            }
            Some(&false) if is_last => {
                return Err(BrokerError::UnsafeNode {
                    path: path.clone(),
                    reason: "not a regular file",
                });
            }
            Some(_) => {}
        }
        descend(&mut parent, component);
    }
    Ok(())
}

fn descend(parent: &mut String, component: &str) {
    if !parent.is_empty() {
        parent.push('/');
    }
    parent.push_str(component);
}
