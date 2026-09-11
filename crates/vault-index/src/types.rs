//! 索引输入与查询结果；只表示派生事实，不授予文件或授权能力。

use crate::{
    FrontmatterWarning, Heading, HeadingNode, NormalizedFrontmatter, OutgoingLink, SourceProfile,
};
use studiolo_core::VaultPath;

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
