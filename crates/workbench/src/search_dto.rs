use serde::{Deserialize, Serialize};

/// A bounded literal search over one immutable Markdown generation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchQuery {
    /// At most 256 Unicode scalars and 1,024 UTF-8 bytes, without control characters.
    pub text: String,
    /// Exact known project root identity, or all indexed documents when absent.
    pub project: Option<String>,
    /// Full path/body literal search or all-fragments path lookup.
    pub mode: SearchMode,
}

/// Search semantics shared by all callers, including keyboard path lookup.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    /// Case-insensitive literal matching against complete paths and body lines.
    FullText,
    /// Every ordinary-space-separated fragment must occur in the complete path.
    Path,
}

/// One source-addressable search result, containing no host filesystem path.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchHit {
    /// Logical document path in the searched generation.
    pub path: String,
    /// M0-selected title, at most 320 Unicode scalars; trailing `…` marks truncation.
    pub title: String,
    /// One-based source line; path matches point to the first line.
    pub line: u32,
    /// At most 320 Unicode scalars, centered near the first matching position.
    pub snippet: String,
    /// Whether the match occurred in a logical path or a source line.
    pub kind: SearchHitKind,
}

/// Source of a literal match.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchHitKind {
    /// Complete document path.
    Path,
    /// One source-text line.
    Content,
}

/// A deterministic, bounded result batch for one immutable generation.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchResults {
    /// First 100 hits in logical path / source-line order.
    pub hits: Vec<SearchHit>,
    /// True only after observing a 101st matching hit.
    pub truncated: bool,
}

/// Rejected search requests or an in-memory matching failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    /// Query size or characters violate the shared search Interface.
    #[error("搜索内容过长或包含控制字符。")]
    InvalidQuery,
    /// Project filter is not an exact identity in the current generation.
    #[error("项目筛选不属于当前视图。")]
    InvalidProject,
    /// Literal matcher construction or in-memory search failed.
    #[error("内存搜索失败：{0}")]
    Engine(String),
}
