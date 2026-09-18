//! 索引输入与查询结果；只表示派生事实，不授予文件或授权能力。

use std::fmt;

use crate::{
    FrontmatterWarning, Heading, HeadingNode, NormalizedFrontmatter, OutgoingLink, SourceProfile,
};
use studiolo_core::VaultPath;

/// A Markdown note supplied to an index rebuild.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownDocument {
    path: VaultPath,
    contents: String,
    unavailable: Option<DocumentUnavailable>,
}

impl MarkdownDocument {
    /// Creates an input note without touching the file system.
    #[must_use]
    pub fn new(path: VaultPath, contents: impl Into<String>) -> Self {
        Self {
            path,
            contents: contents.into(),
            unavailable: None,
        }
    }

    /// 登记一篇读不进来的笔记：它仍然是存在的逻辑路径，只是正文本轮缺席。
    ///
    /// 这是符号链接与超大笔记的降级入口——调用方给出逻辑路径和原因，索引把它记成可查询的
    /// 诊断，而不是让整批构建失败。
    #[must_use]
    pub fn unavailable(path: VaultPath, reason: DocumentUnavailable) -> Self {
        Self {
            path,
            contents: String::new(),
            unavailable: Some(reason),
        }
    }

    /// 返回本篇的降级原因；正常笔记为 `None`。
    #[must_use]
    pub fn unavailable_reason(&self) -> Option<DocumentUnavailable> {
        self.unavailable
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

/// 一篇笔记被降级为诊断的原因。
///
/// 故意只用闭合枚举、不带任何自由字符串：诊断会被渲染给用户，携带 `io::Error` 文案就可能
/// 把 Vault 之外的物理路径漏出去。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DocumentUnavailable {
    /// 路径上出现符号链接、硬链接或其他特殊文件。
    Symlink,
    /// 笔记超过调用方的单文读取上限。
    TooLarge,
    /// 正文不是 UTF-8，或读取本身失败。
    Unreadable,
}

impl fmt::Display for DocumentUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Symlink => "路径上存在符号链接或特殊文件，正文未索引",
            Self::TooLarge => "笔记超过单文读取上限，正文未索引",
            Self::Unreadable => "笔记无法读取或不是 UTF-8，正文未索引",
        };
        formatter.write_str(text)
    }
}

/// 一条降级诊断；只携带逻辑 Vault 路径。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexDiagnostic {
    /// 逻辑路径；不含 Vault 根、挂载点等物理前缀。
    pub path: VaultPath,
    /// 降级原因。
    pub reason: DocumentUnavailable,
}

impl fmt::Display for IndexDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}：{}", self.path, self.reason)
    }
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
    /// 本篇正文缺席的原因；`Some` 表示这是降级诊断项而不是完整解析结果。
    pub unavailable: Option<DocumentUnavailable>,
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
