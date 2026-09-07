use serde::Serialize;

/// One-based location in an input Markdown document.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SourceLocation {
    /// Logical Vault path, never a host filesystem path.
    pub path: String,
    /// One-based source line.
    pub line: u32,
}

/// An explicitly derived value and its evidence; unknown values have neither.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Sourced<T> {
    /// Known value, or `None` when evidence is absent or ambiguous.
    pub value: Option<T>,
    /// Evidence for the known value.
    pub source: Option<SourceLocation>,
}

impl<T> Sourced<T> {
    pub(crate) fn unknown() -> Self {
        Self {
            value: None,
            source: None,
        }
    }

    pub(crate) fn known(value: T, source: SourceLocation) -> Self {
        Self {
            value: Some(value),
            source: Some(source),
        }
    }
}

/// Project facts derived only from current, recognized project documents.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ProjectSummary {
    /// Stable logical identity, equal to the project root.
    pub id: String,
    /// Folder name below `项目/`.
    pub name: String,
    /// Logical project directory, without a trailing slash.
    pub root: String,
    /// Explicit lifecycle declaration; never inferred from activity.
    pub lifecycle: Sourced<String>,
    /// Explicit current version from INDEX.
    pub version: Sourced<String>,
    /// Non-completed task rows in TASKS section ①.
    pub active_tasks: Sourced<u64>,
    /// Open BUG count, unknown if any BUG status cannot be classified.
    pub open_bugs: Sourced<u64>,
    /// First roadmap milestone not explicitly completed.
    pub next_milestone: Sourced<String>,
    /// `standard`, `partial`, or `unfiled`, describing document structure only.
    pub profile: String,
    /// INDEX location, when an INDEX exists.
    pub source: Option<SourceLocation>,
    /// Conservative parsing and missing-evidence notices.
    pub diagnostics: Vec<String>,
}

/// A current, explicit user action with a source link.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ActionItem {
    /// Stable identity from source path, line, and action kind.
    pub id: String,
    /// Project root identity.
    pub project_id: String,
    /// Project folder name.
    pub project_name: String,
    /// 待验收、待拍板、待重启、待外部方或待用户操作。
    pub kind: String,
    /// Source-derived action text.
    pub title: String,
    /// Exact evidence location.
    pub source: SourceLocation,
}

/// Navigable Markdown note metadata.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DocumentSummary {
    /// Logical Vault path.
    pub path: String,
    /// M0 VaultIndex-selected display title.
    pub title: String,
}

/// Unmodified source text for a navigable note.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DocumentView {
    /// Logical Vault path.
    pub path: String,
    /// M0 VaultIndex-selected display title.
    pub title: String,
    /// Exact input Markdown, not rendered HTML.
    pub content: String,
}

/// Owned, deterministic view of one complete input generation.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct WorkbenchSnapshot {
    /// Projects sorted by logical root.
    pub projects: Vec<ProjectSummary>,
    /// Actions sorted by project, source path, source line, and kind.
    pub inbox: Vec<ActionItem>,
    /// All supplied Markdown documents sorted by logical path.
    pub documents: Vec<DocumentSummary>,
    /// Generation-wide parsing notices.
    pub diagnostics: Vec<String>,
}
