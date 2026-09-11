//! 每轮先验证完整成员计划，再逐项读固定文件；路径计划变化不触发额外探索。

use std::collections::BTreeSet;

use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{BrokerError, ReadNote};
use studiolo_workbench::{CargoManifest, cargo_manifest_paths};

use super::Failure;

const ROOT_MANIFEST: &str = "Cargo.toml";
const MAX_MANIFESTS: usize = 65;
const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;

pub(super) struct Captured {
    pub manifest: CargoManifest,
    revision: Revision,
}

impl Captured {
    pub(super) fn same(&self, other: &Self) -> bool {
        self.manifest.path == other.manifest.path
            && self.manifest.content == other.manifest.content
            && self.revision == other.revision
    }
}

pub(super) fn capture(
    read: &mut impl FnMut(&VaultPath, u64) -> Result<ReadNote, BrokerError>,
    previous: Option<&[Captured]>,
) -> Result<Vec<Captured>, Failure> {
    let mut bytes = 0;
    let root = read_manifest(
        read,
        &logical_path(ROOT_MANIFEST)?,
        &mut bytes,
        previous.is_some(),
    )?;
    if let Some(previous) = previous {
        let before = previous
            .iter()
            .find(|item| item.manifest.path == ROOT_MANIFEST);
        if before.is_none_or(|before| !before.same(&root)) {
            return Err(Failure::unavailable(
                "根 Cargo 清单在两轮之间变化，请重新读取。",
            ));
        }
    }
    let paths =
        validated_plan(cargo_manifest_paths(&root.manifest.content).map_err(Failure::from)?)?;
    if let Some(previous) = previous
        && !paths
            .iter()
            .map(VaultPath::as_str)
            .eq(previous.iter().map(|item| item.manifest.path.as_str()))
    {
        return Err(Failure::unavailable("Cargo 成员清单计划在两轮之间变化。"));
    }
    let mut result = Vec::with_capacity(paths.len());
    result.push(root);
    for path in paths {
        if path.as_str() != ROOT_MANIFEST {
            result.push(read_manifest(read, &path, &mut bytes, previous.is_some())?);
        }
    }
    result.sort_by(|left, right| left.manifest.path.cmp(&right.manifest.path));
    Ok(result)
}

fn read_manifest(
    read: &mut impl FnMut(&VaultPath, u64) -> Result<ReadNote, BrokerError>,
    path: &VaultPath,
    bytes: &mut usize,
    verifying: bool,
) -> Result<Captured, Failure> {
    let remaining = MAX_TOTAL_BYTES.saturating_sub(*bytes) as u64;
    let note = read(path, MAX_FILE_BYTES.min(remaining)).map_err(|_| {
        Failure::unavailable("Cargo 清单缺失、不安全、不可读、超出预算或读取时发生变化。")
    })?;
    if note.path != *path || note.content.len() as u64 > MAX_FILE_BYTES {
        return Err(Failure::unavailable(
            "Cargo 清单读取结果超出固定路径或单文件预算。",
        ));
    }
    *bytes = bytes
        .checked_add(note.content.len())
        .filter(|bytes| *bytes <= MAX_TOTAL_BYTES)
        .ok_or_else(|| Failure::unavailable("Cargo 清单超过每轮 2 MiB 正文预算。"))?;
    let content = String::from_utf8(note.content).map_err(|_| {
        if verifying {
            Failure::unavailable("Cargo 清单在复核读取时变为无效 UTF-8，请重新读取。")
        } else {
            Failure::invalid("Cargo 清单必须是有效 UTF-8 文本。")
        }
    })?;
    Ok(Captured {
        manifest: CargoManifest {
            path: path.to_string(),
            content,
        },
        revision: note.revision,
    })
}

fn validated_plan(paths: Vec<String>) -> Result<Vec<VaultPath>, Failure> {
    if paths.is_empty()
        || paths.len() > MAX_MANIFESTS
        || !paths.iter().any(|path| path == ROOT_MANIFEST)
        || paths.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(Failure::invalid(
            "Cargo 清单计划须包含根且唯一排序，最多 65 个文件。",
        ));
    }
    let mut folded = BTreeSet::new();
    paths
        .into_iter()
        .map(|path| {
            if !folded.insert(path.to_lowercase()) {
                return Err(Failure::invalid("Cargo 清单计划存在大小写名称冲突。"));
            }
            logical_path(&path)
        })
        .collect()
}

fn logical_path(path: &str) -> Result<VaultPath, Failure> {
    if path != ROOT_MANIFEST {
        let directory = path
            .strip_suffix("/Cargo.toml")
            .ok_or_else(|| Failure::invalid("Cargo 采集计划只允许固定 Cargo.toml 文件。"))?;
        if directory.is_empty() || directory.len() > 1024 || directory.split('/').count() > 16 {
            return Err(Failure::invalid("Cargo 包目录超出 1024 字节或 16 层预算。"));
        }
    }
    let path = VaultPath::parse(path)
        .map_err(|_| Failure::invalid("Cargo 清单计划包含不规范相对路径。"))?;
    if path.as_str().split('/').any(|part| part.starts_with('.')) {
        return Err(Failure::invalid("Cargo 清单计划不允许隐藏目录。"));
    }
    Ok(path)
}
