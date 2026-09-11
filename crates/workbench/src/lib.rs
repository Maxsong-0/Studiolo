//! Pure, rebuildable M1 project views. This Module performs no filesystem,
//! network, process, or clock operations: every displayed fact comes from the
//! supplied documents or explicit Cargo manifest inputs and retains its source location.

#![forbid(unsafe_code)]

mod actions;
mod architecture;
mod architecture_dto;
mod architecture_parse;
mod architecture_scan;
mod bugs;
mod cargo_dependencies;
mod cargo_dto;
mod cargo_manifest;
mod cargo_plan;
mod cargo_spec;
mod derive;
mod dto;
mod markdown;
mod metadata;
mod navigation;
mod navigation_dto;
mod operations;
mod operations_block;
mod operations_compare;
mod operations_dto;
mod operations_parse;
mod search;
mod search_dto;
mod table;
mod tree;
mod tree_dto;
mod tree_validate;

use std::collections::BTreeMap;

use studiolo_core::VaultPath;
use studiolo_vault_index::{
    InMemoryVaultIndex, MarkdownDocument, ParseCache, ParseCacheStats, VaultIndex,
};

pub use architecture_dto::*;
pub use cargo_dependencies::derive_cargo_dependencies;
pub use cargo_dto::*;
pub use cargo_plan::cargo_manifest_paths;
pub use dto::*;
pub use navigation_dto::*;
pub use operations_dto::*;
pub use search_dto::*;
pub use studiolo_vault_index::IndexError;
pub use tree::FileTree;
pub use tree_dto::*;

/// One immutable derived generation, built through the same Interface used by
/// callers and contract tests. Duplicate input paths reject the whole build.
#[derive(Debug)]
pub struct WorkbenchReadModel {
    snapshot: WorkbenchSnapshot,
    documents: BTreeMap<VaultPath, DocumentView>,
    index: InMemoryVaultIndex,
}

impl WorkbenchReadModel {
    /// Builds deterministic views and reuses M0 parsing without any I/O.
    pub fn build(documents: &[MarkdownDocument]) -> Result<Self, IndexError> {
        let mut index = InMemoryVaultIndex::new();
        index.rebuild(documents)?;
        Self::from_index(documents, index)
    }

    /// 复用单文解析；内容来自本次输入，全局链接与项目视图重新派生。
    pub fn build_cached(
        documents: &[MarkdownDocument],
        previous: Option<&ParseCache>,
    ) -> Result<(Self, ParseCache, ParseCacheStats), IndexError> {
        let mut index = InMemoryVaultIndex::new();
        let (_, cache, stats) = index.rebuild_cached(documents, previous)?;
        Ok((Self::from_index(documents, index)?, cache, stats))
    }

    fn from_index(
        documents: &[MarkdownDocument],
        index: InMemoryVaultIndex,
    ) -> Result<Self, IndexError> {
        let mut views = BTreeMap::new();
        let mut inputs = BTreeMap::new();
        let mut diagnostics = Vec::new();
        for input in documents {
            let parsed = index
                .document(input.path())?
                .ok_or_else(|| IndexError::NotIndexed(input.path().clone()))?;
            if !parsed.warnings.is_empty() {
                diagnostics.push(format!("{}: frontmatter 存在解析或别名警告", input.path()));
            }
            views.insert(
                input.path().clone(),
                DocumentView {
                    path: input.path().as_str().to_owned(),
                    title: parsed.title.text.clone(),
                    content: input.contents().to_owned(),
                },
            );
            inputs.insert(input.path().as_str().to_owned(), (input, parsed));
        }
        diagnostics.sort();
        let (projects, inbox) = derive::build(&inputs);
        let summaries = views
            .values()
            .map(|view| DocumentSummary {
                path: view.path.clone(),
                title: view.title.clone(),
            })
            .collect();
        Ok(Self {
            snapshot: WorkbenchSnapshot {
                projects,
                inbox,
                documents: summaries,
                diagnostics,
            },
            documents: views,
            index,
        })
    }

    /// Returns an owned copy; caller changes cannot mutate this generation.
    #[must_use]
    pub fn snapshot(&self) -> WorkbenchSnapshot {
        self.snapshot.clone()
    }

    /// Returns exact input text, or `None` for a path outside this generation.
    #[must_use]
    pub fn document(&self, path: &VaultPath) -> Option<DocumentView> {
        self.documents.get(path).cloned()
    }

    /// Searches only this immutable generation. Full text and path switching
    /// share the same document set, query limits, and exact project scope.
    pub fn search(&self, query: &SearchQuery) -> Result<SearchResults, SearchError> {
        search::search(&self.documents, &self.snapshot.projects, query)
    }

    /// Derives outline, wikilink targets, and backlinks from the same M0 index.
    /// Missing notes return `None`; navigation truncation is explicit.
    #[must_use]
    pub fn navigation(&self, path: &VaultPath) -> Option<DocumentNavigation> {
        navigation::navigation(&self.index, path)
    }

    /// 从本代目录声明派生项目运作图首片；未知项目不产生视图。
    #[must_use]
    pub fn project_operations(&self, project: &str) -> Option<ProjectOperations> {
        self.snapshot
            .projects
            .iter()
            .find(|known| known.id == project)?;
        Some(operations::derive(&self.documents, project))
    }

    /// 从本代可选表格派生设计声明；不表示实际依赖或正在执行的任务。
    #[must_use]
    pub fn project_architecture(&self, project: &str) -> Option<ProjectArchitecture> {
        self.snapshot
            .projects
            .iter()
            .find(|known| known.id == project)?;
        Some(architecture::derive(&self.documents, project))
    }
}
