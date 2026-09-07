//! Read-only, rebuildable views over Markdown notes in a Studiolo Vault.
//!
//! Callers supply note text explicitly. This crate never opens or writes the
//! canonical Vault itself.

#![forbid(unsafe_code)]

mod frontmatter;
mod link_parse;
mod link_resolve;
mod links;
mod markdown;
mod markdown_scan;
mod source;

use std::collections::BTreeMap;

use studiolo_core::VaultPath;

pub use frontmatter::{
    FRONTMATTER_KEY_ALIASES, FRONTMATTER_SPECIAL_KEY_ALIASES, FrontmatterEntry, FrontmatterValue,
    FrontmatterWarning, KeyAliasGroup, NormalizedFrontmatter, NormalizedStatus,
};
pub use links::{
    AmbiguousLink, Backlink, LinkHealth, LinkIssue, LinkResolution, OutgoingLink, ResolutionRule,
    WikilinkParseError, WikilinkTarget,
};
pub use markdown::{Heading, HeadingNode};
pub use source::{SourceProfile, SourceProvenance};

/// A Markdown note supplied to an index rebuild.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownDocument {
    path: VaultPath,
    contents: String,
}

impl MarkdownDocument {
    /// Creates an input note without touching the file system.
    #[must_use]
    pub fn new(path: VaultPath, contents: impl Into<String>) -> Self {
        Self {
            path,
            contents: contents.into(),
        }
    }

    /// Returns the logical Vault path.
    #[must_use]
    pub fn path(&self) -> &VaultPath {
        &self.path
    }

    /// Returns the original Markdown text.
    #[must_use]
    pub fn contents(&self) -> &str {
        &self.contents
    }
}

/// Counts attached to a completed in-memory rebuild.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexGeneration {
    /// Monotonically increasing in-process generation number.
    pub generation: u64,
    /// Number of indexed notes.
    pub documents: usize,
    /// Number of parsed wikilinks.
    pub links: usize,
}

/// Error returned by the public index seam.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    /// The same logical path appeared twice in one rebuild.
    #[error("duplicate document path: {0}")]
    DuplicatePath(VaultPath),
    /// A query addressed a path absent from the current generation.
    #[error("document is not indexed: {0}")]
    NotIndexed(VaultPath),
}

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
            if replacement.contains_key(&input.path) {
                return Err(IndexError::DuplicatePath(input.path.clone()));
            }
            replacement.insert(input.path.clone(), parse_document(input));
        }

        let paths: Vec<_> = replacement.keys().cloned().collect();
        for path in &paths {
            let targets: Vec<_> = replacement[path]
                .outgoing_links
                .iter()
                .map(|link| link.target.clone())
                .collect();
            let resolutions: Vec<_> = targets
                .iter()
                .map(|target| links::resolve(&replacement, path, target))
                .collect();
            if let Some(document) = replacement.get_mut(path) {
                for (link, resolution) in document.outgoing_links.iter_mut().zip(resolutions) {
                    link.resolution = resolution;
                }
            }
        }

        let (backlinks, link_health) = links::derive_links(&replacement);

        self.generation.generation = self.generation.generation.saturating_add(1);
        self.generation.documents = replacement.len();
        self.generation.links = replacement
            .values()
            .map(|document| document.outgoing_links.len())
            .sum();
        self.documents = replacement;
        self.backlinks = backlinks;
        self.link_health = link_health;
        Ok(self.generation)
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

/// Parsed, derived view of one Markdown note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedDocument {
    /// Logical source path.
    pub path: VaultPath,
    /// Display title selected from frontmatter, first H1, then filename.
    pub title: DocumentTitle,
    /// Read-side normalized frontmatter with raw spellings retained.
    pub frontmatter: NormalizedFrontmatter,
    /// Flat headings in source order.
    pub headings: Vec<Heading>,
    /// Nested heading outline.
    pub heading_tree: Vec<HeadingNode>,
    /// Wikilinks parsed from Markdown with their generation-local resolutions.
    pub outgoing_links: Vec<OutgoingLink>,
    /// Advisory origin and classification facts.
    pub source_profile: SourceProfile,
    /// Non-fatal parsing diagnostics.
    pub warnings: Vec<ParseWarning>,
}

/// A display title and the rule that selected it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentTitle {
    /// Unescaped title text.
    pub text: String,
    /// Selection rule used for the title.
    pub source: TitleSource,
}

/// Ordered fallback used to select a note title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleSource {
    /// The normalized `title` frontmatter value.
    Frontmatter,
    /// The first level-one Markdown heading.
    FirstHeading,
    /// The filename without `.md`.
    FileName,
}

/// Non-fatal document parsing diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseWarning {
    /// A frontmatter parser or normalization warning.
    Frontmatter(FrontmatterWarning),
}

fn parse_document(input: &MarkdownDocument) -> IndexedDocument {
    let parts = markdown::split_frontmatter(&input.contents);
    let frontmatter = frontmatter::normalize(parts.yaml, parts.closed);
    let headings = markdown::parse_headings(parts.body, parts.body_first_line);
    let title = select_title(&input.path, &frontmatter, &headings);
    let heading_tree = markdown::build_heading_tree(&headings);
    let outgoing_links = links::parse_outgoing(parts.body, parts.body_first_line);
    let source_profile = source::profile(&input.path, &frontmatter);
    let warnings = frontmatter
        .warnings
        .iter()
        .cloned()
        .map(ParseWarning::Frontmatter)
        .collect();

    IndexedDocument {
        path: input.path.clone(),
        title,
        frontmatter,
        headings,
        heading_tree,
        outgoing_links,
        source_profile,
        warnings,
    }
}

fn select_title(
    path: &VaultPath,
    frontmatter: &NormalizedFrontmatter,
    headings: &[Heading],
) -> DocumentTitle {
    if let Some(FrontmatterValue::Text(title)) = frontmatter.get("title") {
        return DocumentTitle {
            text: title.clone(),
            source: TitleSource::Frontmatter,
        };
    }
    if let Some(heading) = headings.iter().find(|heading| heading.level == 1) {
        return DocumentTitle {
            text: heading.text.clone(),
            source: TitleSource::FirstHeading,
        };
    }
    let filename = path.as_str().rsplit('/').next().unwrap_or(path.as_str());
    DocumentTitle {
        text: filename.strip_suffix(".md").unwrap_or(filename).to_owned(),
        source: TitleSource::FileName,
    }
}
