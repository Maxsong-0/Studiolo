//! 纯内存文件树；在构建期校验完整批次并建立直属父子索引。

use std::collections::{BTreeMap, BTreeSet};

use studiolo_vault_index::MarkdownDocument;

use crate::tree_validate::{parent, validate, visible_path};
use crate::{TreeEntry, TreeError, TreeInput, TreeKind, TreePage};

const PAGE_SIZE: usize = 200;

/// 一次不可变扫描的文件树，不持有源文件正文或任何 I/O 能力。
///
/// 构建时建立父子索引；分页只查指定目录并克隆至多 200 条，不重新全库扫描。
#[derive(Clone, Debug)]
pub struct FileTree {
    directories: BTreeMap<String, Vec<TreeEntry>>,
    files: BTreeSet<String>,
}

impl FileTree {
    /// 从完整清单和同一批 Markdown 正文构建，不读取或修改任何源文件。
    ///
    /// 所有路径须可见、规范且没有重复或大小写碰撞；除隐含根目录外，父目录必须
    /// 显式存在。目录没有大小，文件必须有大小；Markdown 文件集合及 UTF-8 字节数
    /// 必须与正文批次完全一致。任何不一致均以 `InvalidInput` 拒绝整个构建。
    pub fn build(entries: &[TreeInput], documents: &[MarkdownDocument]) -> Result<Self, TreeError> {
        validate(entries, documents)?;
        let mut directories = BTreeMap::from([(String::new(), Vec::new())]);
        let mut files = BTreeSet::new();
        let mut counts = BTreeMap::<&str, usize>::new();
        for entry in entries {
            let path = entry.path.as_str();
            *counts.entry(parent(path)).or_default() += 1;
            if entry.kind == TreeKind::Directory {
                directories.insert(path.to_owned(), Vec::new());
            } else {
                files.insert(path.to_owned());
            }
        }
        for entry in entries {
            let path = entry.path.as_str();
            let children = directories
                .get_mut(parent(path))
                .ok_or(TreeError::InvalidInput)?;
            children.push(TreeEntry {
                path: path.to_owned(),
                name: path.rsplit('/').next().unwrap_or(path).to_owned(),
                kind: entry.kind,
                size_bytes: entry.size_bytes.map(|size| size.to_string()),
                child_count: (entry.kind == TreeKind::Directory)
                    .then(|| counts.get(path).copied().unwrap_or_default()),
            });
        }
        for children in directories.values_mut() {
            children.sort_by(|left, right| {
                (left.kind != TreeKind::Directory)
                    .cmp(&(right.kind != TreeKind::Directory))
                    .then_with(|| left.path.cmp(&right.path))
            });
        }
        Ok(Self { directories, files })
    }

    /// 读取指定目录的直属子节点；空路径表示根，其余必须是可见规范目录路径。
    ///
    /// 起点可为任何有效位置，不要求是 200 的倍数；非空目录不接受等于总数的起点。
    /// 空目录仅允许零。路径不规范、不存在、非目录和分页越界分别返回明确错误。
    pub fn page(&self, path: &str, offset: usize) -> Result<TreePage, TreeError> {
        if !path.is_empty() && !visible_path(path) {
            return Err(TreeError::InvalidPath);
        }
        let children = self.directories.get(path).ok_or_else(|| {
            if self.files.contains(path) {
                TreeError::NotDirectory
            } else {
                TreeError::NotFound
            }
        })?;
        let total = children.len();
        if offset != 0 && offset >= total {
            return Err(TreeError::InvalidOffset);
        }
        let end = offset.saturating_add(PAGE_SIZE).min(total);
        Ok(TreePage {
            path: path.to_owned(),
            offset,
            total,
            entries: children[offset..end].to_vec(),
            next_offset: (end < total).then_some(end),
        })
    }
}
