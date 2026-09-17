//! 目录名有界先验与完整流式核验；陌生前缀保留，自有前缀畸形则整批拒绝。
use super::{child, logical};
use crate::blob::{
    OBJECT_LIMIT, STAGE_BYTES, STAGE_COUNT,
    file::{ContentProof, Stamp},
};
use crate::{BrokerError, FsVaultBroker, VaultPath, VaultUuid};
use cap_std::fs::{Dir, MetadataExt as _};

const PREFIX: &str = ".studiolo-blob-";
// 生成器先扫描 50000 项再创建一个临时对象，崩溃残留可令目录达到 50001 项。
// 只扩大清理的封闭枚举，不扩大通用读取或新对象生成边界。
const DIRECTORY_ENTRIES: usize = 50_001;

#[derive(PartialEq, Eq)]
pub(super) struct Inventory {
    pub(super) names: Vec<String>,
    pub(super) stamp: Stamp,
}

pub(super) fn inventory(root: &Dir) -> Result<Inventory, BrokerError> {
    let before = Stamp::new(
        &root
            .dir_metadata()
            .map_err(|e| BrokerError::io("stat blob cleanup directory", e))?,
    );
    let mut names = Vec::new();
    for entry in root
        .entries()
        .map_err(|e| BrokerError::io("list blob cleanup directory", e))?
    {
        if names.len() >= DIRECTORY_ENTRIES {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect blob cleanup entry", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        names.push(name);
    }
    names.sort();
    let after = Stamp::new(
        &root
            .dir_metadata()
            .map_err(|e| BrokerError::io("restat blob cleanup directory", e))?,
    );
    if before != after || names.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(BrokerError::CaptureChanged);
    }
    Ok(Inventory {
        names,
        stamp: after,
    })
}

pub(super) fn owned(name: &str) -> Result<bool, BrokerError> {
    let Some(tail) = name.strip_prefix(PREFIX) else {
        return Ok(false);
    };
    let id = tail
        .strip_suffix(".tmp")
        .filter(|value| value.len() == 36)
        .ok_or(BrokerError::CorruptWriterState)?;
    let parsed = id
        .parse::<VaultUuid>()
        .map_err(|_| BrokerError::CorruptWriterState)?;
    if parsed.to_string() != id {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(true)
}

pub(super) fn candidates(
    root: &Dir,
    inventory: &Inventory,
) -> Result<Vec<(VaultPath, ContentProof)>, BrokerError> {
    collision_check(&inventory.names)?;
    let mut pending = Vec::new();
    let mut total = 0u64;
    // 先检查整个目录的个数、逻辑大小、类型，再为候选逐个读完整字节。
    for name in &inventory.names {
        if !owned(name)? {
            continue;
        }
        if pending.len() >= STAGE_COUNT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let path = logical(name)?;
        let metadata = root
            .symlink_metadata(name)
            .map_err(|e| BrokerError::io("stat blob cleanup candidate", e))?;
        FsVaultBroker::validate_file_metadata(&path, &metadata)?;
        if metadata.nlink() != 1 || metadata.len() > OBJECT_LIMIT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        total = total
            .checked_add(metadata.len())
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if total > STAGE_BYTES {
            return Err(BrokerError::WriterLimitExceeded);
        }
        pending.push((path, Stamp::new(&metadata)));
    }
    let mut result = Vec::new();
    for (path, stamp) in pending {
        let proof = child::scan(root, &path, &stamp)?;
        if proof.node.file != stamp {
            return Err(BrokerError::CaptureChanged);
        }
        result.push((path, proof));
    }
    Ok(result)
}

pub(super) fn collision_check(names: &[String]) -> Result<(), BrokerError> {
    let mut seen = std::collections::BTreeMap::new();
    for name in names {
        let key = crate::collision_component_key(name);
        if let Some(previous) = seen.insert(key, name) {
            return Err(BrokerError::PathCollision {
                requested: logical(previous).or_else(|_| logical(name))?,
                existing: previous.clone(),
            });
        }
    }
    Ok(())
}
