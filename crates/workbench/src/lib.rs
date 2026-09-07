//! Pure, rebuildable M1 project views. This Module performs no filesystem,
//! network, process, or clock operations: every displayed fact comes from the
//! supplied Markdown generation and retains its source location.

#![forbid(unsafe_code)]

mod actions;
mod bugs;
mod derive;
mod dto;
mod markdown;
mod metadata;
mod navigation;
mod navigation_dto;
mod search;
mod search_dto;
mod table;
mod tree;
mod tree_dto;
mod tree_validate;

use std::collections::BTreeMap;

use studiolo_core::VaultPath;
use studiolo_vault_index::{InMemoryVaultIndex, MarkdownDocument, VaultIndex};

pub use dto::*;
pub use navigation_dto::*;
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
}
