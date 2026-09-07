use serde::Serialize;

use crate::SourceLocation;

/// Source-linked navigation for one immutable document generation.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DocumentNavigation {
    /// Flat source-order outline, limited to 200 entries.
    pub outline: Vec<OutlineEntry>,
    /// Parsed links in source order, limited to 200 entries.
    pub outgoing: Vec<NavigationLink>,
    /// Resolved page-level inbound links from the entire Vault, limited to 200.
    pub backlinks: Vec<BacklinkView>,
    /// A list, candidate list, or display string reached its display limit.
    pub truncated: bool,
}

/// One heading in the source document.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct OutlineEntry {
    /// Plain heading text, at most 320 Unicode scalars.
    pub title: String,
    /// Markdown heading level from 1 through 6.
    pub level: u8,
    /// Exact source heading line.
    pub source: SourceLocation,
}

/// One parsed wikilink with an explicit, non-guessing resolution result.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NavigationLink {
    /// Display alias, or the target and optional heading.
    pub label: String,
    /// Original text between `[[` and `]]`, display-limited to 320 scalars.
    pub raw: String,
    /// Exact line containing the link.
    pub source: SourceLocation,
    /// `resolved`, `ambiguous`, `missing`, `missing_heading`, or `ambiguous_heading`.
    pub status: String,
    /// Only real destinations; limited to 50, with complete logical paths.
    pub targets: Vec<LinkDestination>,
}

/// A real, selectable note or heading destination.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LinkDestination {
    /// Complete logical Vault path; never truncated or converted to a host path.
    pub path: String,
    /// One-based target line, or 1 for a page-only link.
    pub line: u32,
    /// Display title of the target document and optional heading.
    pub title: String,
}

/// One real inbound page link, preserving its source context.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct BacklinkView {
    /// Display title of the source document.
    pub title: String,
    /// Exact source line containing the inbound link.
    pub source: SourceLocation,
    /// Original source line, display-limited to 320 Unicode scalars.
    pub context: String,
}
