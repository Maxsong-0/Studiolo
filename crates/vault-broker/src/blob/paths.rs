//! 有界逐级路径与缺失证据；父链不依赖遍历所有仓库文件。
use super::file::Stamp;
use crate::{
    BrokerError, VaultPath,
    writer::{Identity, directory},
};
use cap_std::fs::{Dir, MetadataExt as _};
pub(crate) fn path_budget(path: &VaultPath) -> Result<(), BrokerError> {
    if path.as_str().len() > 4096 || path.as_str().split('/').count() > 64 {
        Err(BrokerError::WriterLimitExceeded)
    } else {
        Ok(())
    }
}
pub(crate) fn parents(root: &Dir, path: &VaultPath) -> Result<Vec<(String, Stamp)>, BrokerError> {
    path_budget(path)?;
    let device = Identity::directory(root)?.dev;
    let mut prefix = String::new();
    let mut result = Vec::new();
    for component in path.as_str().split('/') {
        let dir = directory(root, if prefix.is_empty() { "." } else { &prefix })?;
        let metadata = dir
            .dir_metadata()
            .map_err(|e| BrokerError::io("stat blob parent", e))?;
        if metadata.dev() != device {
            return Err(BrokerError::WriterBindingMismatch);
        }
        collision(&dir, component, path)?;
        result.push((prefix.clone(), Stamp::new(&metadata)));
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(component);
    }
    Ok(result)
}
pub(crate) fn collision(root: &Dir, name: &str, logical: &VaultPath) -> Result<(), BrokerError> {
    let key = crate::collision_component_key(name);
    for (index, entry) in root
        .entries()
        .map_err(|e| BrokerError::io("enumerate blob names", e))?
        .enumerate()
    {
        if index >= 50_000 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let name_on_disk = entry
            .map_err(|e| BrokerError::io("inspect blob name", e))?
            .file_name();
        let existing = name_on_disk
            .to_str()
            .ok_or_else(|| BrokerError::UnsafeNode {
                path: logical.clone(),
                reason: "non-UTF-8 source sibling",
            })?;
        if existing != name && crate::collision_component_key(existing) == key {
            return Err(BrokerError::PathCollision {
                requested: logical.clone(),
                existing: existing.to_owned(),
            });
        }
    }
    Ok(())
}
pub(crate) fn missing(
    root: &Dir,
    path: &VaultPath,
) -> Result<Option<Vec<(String, Stamp)>>, BrokerError> {
    path_budget(path)?;
    let device = Identity::directory(root)?.dev;
    let mut prefix = String::new();
    let mut parents = Vec::new();
    for component in path.as_str().split('/') {
        let parent = directory(root, if prefix.is_empty() { "." } else { &prefix })?;
        let before = parent
            .dir_metadata()
            .map_err(|e| BrokerError::io("stat absence parent", e))?;
        if before.dev() != device {
            return Err(BrokerError::WriterBindingMismatch);
        }
        let stamp = Stamp::new(&before);
        collision(&parent, component, path)?;
        parents.push((prefix.clone(), stamp.clone()));
        match parent.symlink_metadata(component) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if Stamp::new(
                    &parent
                        .dir_metadata()
                        .map_err(|e| BrokerError::io("restat absence parent", e))?,
                ) != stamp
                {
                    return Err(BrokerError::CaptureChanged);
                }
                return Ok(Some(parents));
            }
            Err(error) => return Err(BrokerError::io("inspect absence", error)),
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(BrokerError::UnsafeNode {
                    path: path.clone(),
                    reason: "linked absence path",
                });
            }
            Ok(_) => {}
        }
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(component);
    }
    Ok(None)
}
