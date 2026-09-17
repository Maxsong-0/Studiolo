//! 第一阶段只读取名字与元信息；不打开正文，全部名字先核碰撞再筛选。
use super::{SyncReadSelection, paths, state::*};
use crate::{
    BrokerError, VaultPath,
    blob::{
        file::{NodeProof, Stamp},
        observed,
    },
    writer::{Identity, directory},
};
use cap_std::fs::{Dir, MetadataExt as _};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn capture(
    root: &Dir,
    private: bool,
    selection: &SyncReadSelection,
    required: &[VaultPath],
    mut index: paths::Index,
) -> Result<Survey, BrokerError> {
    let mut directories: BTreeMap<String, Directory> = BTreeMap::new();
    let mut files = BTreeMap::new();
    let mut pending = vec![String::new()];
    let mut inspected = 0_usize;
    let mut bytes = 0_usize;
    while let Some(prefix) = pending.pop() {
        if directories.len() >= 4096 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let dir = directory(root, if prefix.is_empty() { "." } else { &prefix })?;
        #[cfg(test)]
        if !prefix.is_empty() {
            super::tests::hook(4);
        }
        let before = observed::directory_stamp(root, &prefix, private)?;
        if observed::opened_directory_stamp(&dir, private)? != before {
            return Err(BrokerError::CaptureChanged);
        }
        let mut names = BTreeSet::new();
        let mut entries = Vec::new();
        for entry in dir
            .entries()
            .map_err(|e| BrokerError::io("enumerate sync metadata", e))?
        {
            let entry = entry.map_err(|e| BrokerError::io("read sync metadata entry", e))?;
            inspected = inspected
                .checked_add(1)
                .ok_or(BrokerError::WriterLimitExceeded)?;
            // 触及上限即失败；不为了探测 EOF 再取第 65,537 个名字。
            if inspected >= 65_536 {
                return Err(BrokerError::WriterLimitExceeded);
            }
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(crate::sync_observation::invalid)?;
            let raw = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            };
            bytes = bytes
                .checked_add(raw.len())
                .ok_or(BrokerError::WriterLimitExceeded)?;
            if bytes > 4 * 1024 * 1024 {
                return Err(BrokerError::WriterLimitExceeded);
            }
            let path = VaultPath::parse(raw).map_err(|_| crate::sync_observation::invalid())?;
            index.add(&path)?;
            if !names.insert(name.to_owned()) {
                return Err(BrokerError::CaptureChanged);
            }
            entries.push((name.to_owned(), path));
        }
        // 无论将来是否排除，这一层所有原始名字都已通过同一个索引。
        for (name, path) in entries {
            if selection.excluded(&path) {
                continue;
            }
            let relevant = selection.file_limit(&path).is_some() || selection.traverse(&path);
            if !relevant {
                continue;
            }
            let metadata = dir
                .symlink_metadata(&name)
                .map_err(|e| BrokerError::io("inspect sync metadata", e))?;
            if metadata.file_type().is_symlink() || metadata.dev() != Identity::directory(root)?.dev
            {
                return Err(BrokerError::UnsafeNode {
                    path,
                    reason: "linked or cross-device sync object",
                });
            }
            if metadata.is_dir() {
                if selection.traverse(&path) {
                    if path.as_str().split('/').count() > 64 {
                        return Err(BrokerError::WriterLimitExceeded);
                    }
                    pending.push(path.to_string());
                } else {
                    return Err(crate::sync_observation::invalid());
                }
            } else if !metadata.is_file() || metadata.nlink() != 1 {
                return Err(BrokerError::UnsafeNode {
                    path,
                    reason: "sync observation requires unique regular file",
                });
            } else if selection.needed_directory(&path) {
                return Err(crate::sync_observation::invalid());
            } else if selection.file_limit(&path).is_some() {
                if files.len() >= 4096 {
                    return Err(BrokerError::WriterLimitExceeded);
                }
                crate::readonly::sync::check_file(&metadata, private)?;
                files.insert(path, Stamp::new(&metadata));
            }
        }
        if observed::opened_directory_stamp(&dir, private)? != before
            || observed::directory_stamp(root, &prefix, private)? != before
        {
            return Err(BrokerError::CaptureChanged);
        }
        directories.insert(
            prefix,
            Directory {
                stamp: before,
                names,
            },
        );
    }
    for rule in &selection.folders {
        if !directories.contains_key(rule.path.as_str()) {
            return Err(BrokerError::NotFound(rule.path.clone()));
        }
    }
    let mut paths: BTreeSet<VaultPath> = files.keys().cloned().collect();
    paths.extend(selection.files.iter().map(|rule| rule.path.clone()));
    paths.extend(required.iter().cloned());
    if paths.len() > 4096 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let mut candidates = BTreeMap::new();
    for path in &paths {
        let (parents, absent) = parent_chain(path, &directories)?;
        let candidate = if let Some(stamp) = files.get(path) {
            if absent {
                return Err(BrokerError::CaptureChanged);
            }
            Candidate::Present(NodeProof {
                parents,
                file: stamp.clone(),
            })
        } else {
            if !absent {
                return Err(crate::sync_observation::invalid());
            }
            observed::missing(root, path, &parents, private)?;
            Candidate::Missing(parents)
        };
        candidates.insert(path.clone(), candidate);
    }
    Ok(Survey {
        selection: selection.clone(),
        required: required.to_vec(),
        index,
        paths,
        candidates,
        directories,
    })
}
pub(super) fn parent_chain(
    path: &VaultPath,
    directories: &BTreeMap<String, Directory>,
) -> Result<(Vec<(String, Stamp)>, bool), BrokerError> {
    let mut prefix = String::new();
    let mut parents = Vec::new();
    for component in path.as_str().split('/') {
        let directory = directories
            .get(&prefix)
            .ok_or(BrokerError::CaptureChanged)?;
        parents.push((prefix.clone(), directory.stamp.clone()));
        if !directory.names.contains(component) {
            return Ok((parents, true));
        }
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(component);
    }
    Ok((parents, false))
}
