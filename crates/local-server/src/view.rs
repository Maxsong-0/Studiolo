//! 一次发布整套不可变视图；新鲜度与视图引用在同一短锁内读取或替换。

use std::sync::{Arc, RwLock};

use serde::Serialize;
use studiolo_core::VaultPath;
use studiolo_vault_broker::{ReadEntry, ReadEntryKind, ReadOnlyVault, ScanLimits};
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::{FileTree, TreeInput, TreeKind, WorkbenchReadModel};

use crate::LocalError;

pub(crate) struct Generation {
    pub number: u64,
    pub model: WorkbenchReadModel,
    pub tree: FileTree,
    pub indexed_through: Option<u64>,
    pub invalidation_epoch: Option<u64>,
}

/// 只读观测值；水位字符串不等于保存权限或目标文档 revision。
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Freshness {
    pub stale: bool,
    pub indexed_through: Option<String>,
    pub committed_through: Option<String>,
    pub unresolved: Option<bool>,
    pub watcher_healthy: bool,
    pub reason: Option<&'static str>,
}

pub(crate) struct PublishedView {
    pub generation: Option<Arc<Generation>>,
    pub freshness: Option<Freshness>,
}

#[derive(Clone)]
pub(crate) struct ViewStore(pub Arc<RwLock<PublishedView>>);

pub(crate) struct ViewSnapshot {
    pub generation: Arc<Generation>,
    pub freshness: Option<Freshness>,
}

impl ViewStore {
    pub fn replica(generation: Generation) -> Self {
        Self(Arc::new(RwLock::new(PublishedView {
            generation: Some(Arc::new(generation)),
            freshness: None,
        })))
    }

    pub fn canonical() -> Self {
        Self(Arc::new(RwLock::new(PublishedView {
            generation: None,
            freshness: Some(Freshness {
                stale: true,
                indexed_through: None,
                committed_through: None,
                unresolved: None,
                watcher_healthy: false,
                reason: Some("initializing"),
            }),
        })))
    }

    pub fn snapshot(&self) -> Result<ViewSnapshot, LocalError> {
        let view = self.0.read().map_err(|_| LocalError::Unavailable)?;
        Ok(ViewSnapshot {
            generation: view.generation.clone().ok_or(LocalError::Unavailable)?,
            freshness: view.freshness.clone(),
        })
    }
}

/// 一次报告最多列出的诊断条数；其余只报条数，防止启动错误与刷新日志随副本规模无界增长。
const MAX_REPORTED_DIAGNOSTICS: usize = 20;
/// 每条诊断最多保留的字符数（按 Unicode 标量计），超出部分以省略号代替。
const MAX_DIAGNOSTIC_CHARS: usize = 200;

pub(crate) fn load(vault: &ReadOnlyVault, number: u64) -> Result<Generation, LocalError> {
    let batch = vault.scan(ScanLimits::default())?;
    if !batch.diagnostics.is_empty() {
        return Err(incomplete(&batch.diagnostics));
    }
    build(
        batch.entries,
        batch
            .notes
            .into_iter()
            .map(|note| (note.path, note.content)),
        number,
    )
}

/// 把扫描诊断收成有界摘录：前 [`MAX_REPORTED_DIAGNOSTICS`] 条逐条截断，其余只报条数。
fn incomplete(diagnostics: &[String]) -> LocalError {
    let mut listed = diagnostics
        .iter()
        .take(MAX_REPORTED_DIAGNOSTICS)
        .map(|diagnostic| bounded(diagnostic))
        .collect::<Vec<_>>();
    let hidden = diagnostics.len().saturating_sub(MAX_REPORTED_DIAGNOSTICS);
    if hidden > 0 {
        listed.push(format!("另有 {hidden} 条诊断未列出"));
    }
    LocalError::Incomplete {
        total: diagnostics.len(),
        listed: listed.join("；"),
    }
}

/// 单条诊断按字符边界截断，不会切开多字节字符。
fn bounded(diagnostic: &str) -> String {
    match diagnostic.char_indices().nth(MAX_DIAGNOSTIC_CHARS) {
        Some((end, _)) => format!("{}…", &diagnostic[..end]),
        None => diagnostic.to_owned(),
    }
}

pub(crate) fn build(
    entries: Vec<ReadEntry>,
    notes: impl Iterator<Item = (VaultPath, Vec<u8>)>,
    number: u64,
) -> Result<Generation, LocalError> {
    let entries: Vec<_> = entries
        .into_iter()
        .map(|entry| TreeInput {
            path: entry.path,
            kind: match entry.kind {
                ReadEntryKind::Directory => TreeKind::Directory,
                ReadEntryKind::Markdown => TreeKind::Markdown,
                ReadEntryKind::Attachment => TreeKind::Attachment,
            },
            size_bytes: entry.size_bytes,
        })
        .collect();
    let documents = notes
        .map(|(path, bytes)| {
            let contents =
                String::from_utf8(bytes).map_err(|_| LocalError::Encoding(path.to_string()))?;
            Ok(MarkdownDocument::new(path, contents))
        })
        .collect::<Result<Vec<_>, LocalError>>()?;
    Ok(Generation {
        number,
        model: WorkbenchReadModel::build(&documents)?,
        tree: FileTree::build(&entries, &documents)?,
        indexed_through: None,
        invalidation_epoch: None,
    })
}
