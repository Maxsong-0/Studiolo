//! Read-only, rebuildable views over Markdown notes in a Studiolo Vault.
//!
//! Callers supply note text explicitly. This crate never opens or writes the
//! canonical Vault itself.

#![forbid(unsafe_code)]

mod build;
mod cache;
mod frontmatter;
mod frontmatter_types;
mod frontmatter_yaml;
mod link_parse;
mod link_resolve;
mod links;
mod markdown;
mod markdown_scan;
mod parse;
mod source;
mod types;

use std::collections::BTreeMap;

use studiolo_core::VaultPath;

pub use frontmatter_types::{
    FRONTMATTER_KEY_ALIASES, FRONTMATTER_SPECIAL_KEY_ALIASES, FrontmatterEntry, FrontmatterValue,
    FrontmatterWarning, KeyAliasGroup, NormalizedFrontmatter, NormalizedStatus,
};
pub use links::{
    AmbiguousLink, Backlink, LinkHealth, LinkIssue, LinkResolution, OutgoingLink, ResolutionRule,
    WikilinkParseError, WikilinkTarget,
};
pub use markdown::{Heading, HeadingNode};
pub use source::{SourceProfile, SourceProvenance};

pub use cache::{CacheError, MAX_CACHE_BYTES, PARSE_CACHE_VERSION, ParseCache, ParseCacheStats};
pub use types::{
    DocumentTitle, IndexError, IndexGeneration, IndexedDocument, MarkdownDocument, ParseWarning,
    TitleSource,
};

/// Public query seam for the read-only, derived Vault index.
pub trait VaultIndex: Send + Sync {
    /// Replaces the derived generation with the supplied notes.
    ///
    /// This mutates only in-memory derived data and performs no Vault I/O.
    fn rebuild(&mut self, documents: &[MarkdownDocument]) -> Result<IndexGeneration, IndexError>;

    /// Returns one parsed note, or `None` when it is not indexed.
    fn document(&self, path: &VaultPath) -> Result<Option<IndexedDocument>, IndexError>;

    /// Returns the nested page outline for one note.
    fn heading_tree(&self, path: &VaultPath) -> Result<Vec<HeadingNode>, IndexError>;

    /// Resolves one wikilink using exact-path rules followed by nearest-first lookup.
    fn resolve_link(
        &self,
        from: &VaultPath,
        target: &WikilinkTarget,
    ) -> Result<LinkResolution, IndexError>;

    /// Returns resolved links that point to `to`.
    fn backlinks(&self, to: &VaultPath) -> Result<Vec<Backlink>, IndexError>;

    /// Returns all dangling and genuinely ambiguous links.
    fn link_health(&self) -> Result<LinkHealth, IndexError>;

    /// Returns advisory source facts for one note.
    ///
    /// The result is deliberately not `DataLabels` and must not be used for
    /// authorization.
    fn source_profile(&self, path: &VaultPath) -> Result<SourceProfile, IndexError>;

    /// Returns counters for the current generation.
    fn generation(&self) -> IndexGeneration;
}
/// In-memory M0 adapter for [`VaultIndex`].
#[derive(Debug, Default)]
pub struct InMemoryVaultIndex {
    documents: BTreeMap<VaultPath, IndexedDocument>,
    backlinks: BTreeMap<VaultPath, Vec<Backlink>>,
    link_health: LinkHealth,
    generation: IndexGeneration,
}

impl InMemoryVaultIndex {
    /// Creates an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl VaultIndex for InMemoryVaultIndex {
    fn rebuild(&mut self, documents: &[MarkdownDocument]) -> Result<IndexGeneration, IndexError> {
        let mut replacement = BTreeMap::new();
        for input in documents {
            if replacement.contains_key(input.path()) {
                return Err(IndexError::DuplicatePath(input.path().clone()));
            }
            replacement.insert(
                input.path().clone(),
                parse::ParsedDocument::parse(input).finish(input.path()),
            );
        }
        Ok(self.publish(replacement))
    }

    fn document(&self, path: &VaultPath) -> Result<Option<IndexedDocument>, IndexError> {
        Ok(self.documents.get(path).cloned())
    }

    fn heading_tree(&self, path: &VaultPath) -> Result<Vec<HeadingNode>, IndexError> {
        self.documents
            .get(path)
            .map(|document| document.heading_tree.clone())
            .ok_or_else(|| IndexError::NotIndexed(path.clone()))
    }

    fn resolve_link(
        &self,
        from: &VaultPath,
        target: &WikilinkTarget,
    ) -> Result<LinkResolution, IndexError> {
        if !self.documents.contains_key(from) {
            return Err(IndexError::NotIndexed(from.clone()));
        }
        Ok(links::resolve(&self.documents, from, target))
    }

    fn backlinks(&self, to: &VaultPath) -> Result<Vec<Backlink>, IndexError> {
        Ok(self.backlinks.get(to).cloned().unwrap_or_default())
    }

    fn link_health(&self) -> Result<LinkHealth, IndexError> {
        Ok(self.link_health.clone())
    }

    fn source_profile(&self, path: &VaultPath) -> Result<SourceProfile, IndexError> {
        self.documents
            .get(path)
            .map(|document| document.source_profile.clone())
            .ok_or_else(|| IndexError::NotIndexed(path.clone()))
    }

    fn generation(&self) -> IndexGeneration {
        self.generation
    }
}
