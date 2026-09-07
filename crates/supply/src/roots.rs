//! 白名单只读根：每个允许的目录各开一个 `ReadOnlyVault`，未列入的目录从一开始不进入任何计算。

use std::path::Path;

use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{BrokerError, ReadEntryKind, ReadOnlyVault, ScanLimits};
use studiolo_vault_index::MarkdownDocument;

use crate::SupplyError;

/// 白名单内的一条可见条目，逻辑路径已带白名单前缀。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// 相对于 vault 根的逻辑路径（以白名单前缀开头）。
    pub path: VaultPath,
    /// 条目类型。
    pub kind: EntryKind,
    /// 文件字节数；目录为 `None`。
    pub size_bytes: Option<u64>,
}

/// 条目类型；附件只有元信息。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// 目录。
    Directory,
    /// 已读入内存的 Markdown。
    Markdown,
    /// 附件，不读取内容。
    Attachment,
}

/// 一篇已读入的 Markdown 及其只读 revision。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// 逻辑路径（带白名单前缀）。
    pub path: VaultPath,
    /// 路径与内容指纹（只读命名空间，不是 canonical 身份）。
    pub revision: Revision,
    /// UTF-8 正文。
    pub content: String,
}

/// 一次完整扫描得到的白名单视图；`diagnostics` 非空表示批次不完整，但不拒绝供给。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// 白名单内全部可见条目，按逻辑路径排序。
    pub entries: Vec<Entry>,
    /// 白名单内全部 Markdown。
    pub notes: Vec<Note>,
    /// 逻辑路径与固定原因；不含宿主路径或正文。
    pub diagnostics: Vec<String>,
}

impl Snapshot {
    /// 供 workbench / vault-index 使用的文档集合。
    #[must_use]
    pub fn documents(&self) -> Vec<MarkdownDocument> {
        self.notes
            .iter()
            .map(|note| MarkdownDocument::new(note.path.clone(), note.content.clone()))
            .collect()
    }

    /// 按逻辑路径查找已读入的笔记。
    #[must_use]
    pub fn note(&self, path: &VaultPath) -> Option<&Note> {
        self.notes.iter().find(|note| &note.path == path)
    }
}

struct AllowedRoot {
    prefix: VaultPath,
    vault: ReadOnlyVault,
}

/// 白名单只读根集合。
pub struct SupplyRoots {
    roots: Vec<AllowedRoot>,
}

impl SupplyRoots {
    /// 打开 vault 根下每个白名单目录；任一目录缺失、为链接或前缀不合法即失败关闭。
    pub fn open(vault_root: &Path, allow: &[&str]) -> Result<Self, SupplyError> {
        if allow.is_empty() {
            return Err(SupplyError::InvalidAllowlist("白名单为空"));
        }
        let mut prefixes: Vec<VaultPath> = Vec::new();
        for raw in allow {
            let prefix = VaultPath::parse(raw).map_err(SupplyError::PathSyntax)?;
            if prefix.is_protected()
                || prefix
                    .as_str()
                    .split('/')
                    .any(|component| component.starts_with('.'))
            {
                return Err(SupplyError::InvalidAllowlist("前缀含隐藏或受保护分量"));
            }
            if prefixes.iter().any(|existing| {
                existing == &prefix
                    || prefix.as_str().starts_with(&format!("{existing}/"))
                    || existing.as_str().starts_with(&format!("{prefix}/"))
            }) {
                return Err(SupplyError::InvalidAllowlist("前缀重复或互相嵌套"));
            }
            prefixes.push(prefix);
        }
        let mut roots = Vec::with_capacity(prefixes.len());
        for prefix in prefixes {
            let vault = ReadOnlyVault::open(&vault_root.join(prefix.as_str()))
                .map_err(SupplyError::Broker)?;
            roots.push(AllowedRoot { prefix, vault });
        }
        Ok(Self { roots })
    }

    /// 白名单前缀，按配置顺序。
    pub fn prefixes(&self) -> impl Iterator<Item = &VaultPath> {
        self.roots.iter().map(|root| &root.prefix)
    }

    /// 扫描全部白名单根并把路径映射到带前缀的逻辑路径；每个根分别受限额约束。
    pub fn scan(&self, limits: ScanLimits) -> Result<Snapshot, SupplyError> {
        let mut snapshot = Snapshot::default();
        for root in &self.roots {
            let batch = root.vault.scan(limits).map_err(SupplyError::Broker)?;
            for entry in batch.entries {
                snapshot.entries.push(Entry {
                    path: join(&root.prefix, &entry.path)?,
                    kind: match entry.kind {
                        ReadEntryKind::Directory => EntryKind::Directory,
                        ReadEntryKind::Markdown => EntryKind::Markdown,
                        ReadEntryKind::Attachment => EntryKind::Attachment,
                    },
                    size_bytes: entry.size_bytes,
                });
            }
            for note in batch.notes {
                let path = join(&root.prefix, &note.path)?;
                match String::from_utf8(note.content) {
                    Ok(content) => snapshot.notes.push(Note {
                        path,
                        revision: note.revision,
                        content,
                    }),
                    Err(_) => snapshot.diagnostics.push(format!("{path}: 不是 UTF-8")),
                }
            }
            for diagnostic in batch.diagnostics {
                snapshot
                    .diagnostics
                    .push(format!("{}/{diagnostic}", root.prefix));
            }
        }
        snapshot
            .entries
            .sort_by(|left, right| left.path.cmp(&right.path));
        snapshot
            .notes
            .sort_by(|left, right| left.path.cmp(&right.path));
        Ok(snapshot)
    }

    /// 直接读取白名单内一篇笔记的当前内容；白名单外一律 `OutsideAllowlist`。
    pub fn read_note(&self, path: &VaultPath, max_bytes: u64) -> Result<Note, SupplyError> {
        let (root, relative) = self.locate(path)?;
        let note = root
            .vault
            .read_file(&relative, max_bytes)
            .map_err(|error| match error {
                BrokerError::NotFound(_) => SupplyError::NotFound,
                other => SupplyError::Broker(other),
            })?;
        let content = String::from_utf8(note.content).map_err(|_| SupplyError::NotUtf8)?;
        Ok(Note {
            path: path.clone(),
            revision: note.revision,
            content,
        })
    }

    /// 判断逻辑路径是否落在某个白名单前缀之下（不访问文件系统）。
    #[must_use]
    pub fn allows(&self, path: &VaultPath) -> bool {
        self.locate(path).is_ok()
    }

    fn locate(&self, path: &VaultPath) -> Result<(&AllowedRoot, VaultPath), SupplyError> {
        for root in &self.roots {
            let prefix = format!("{}/", root.prefix);
            if let Some(rest) = path.as_str().strip_prefix(&prefix) {
                let relative = VaultPath::parse(rest).map_err(SupplyError::PathSyntax)?;
                return Ok((root, relative));
            }
        }
        Err(SupplyError::OutsideAllowlist)
    }
}

fn join(prefix: &VaultPath, relative: &VaultPath) -> Result<VaultPath, SupplyError> {
    VaultPath::parse(format!("{prefix}/{relative}")).map_err(SupplyError::PathSyntax)
}
