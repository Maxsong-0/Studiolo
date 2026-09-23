//! 索引输入与查询结果；只表示派生事实，不授予文件或授权能力。

use std::fmt;

use crate::{
    FrontmatterWarning, Heading, HeadingNode, NormalizedFrontmatter, OutgoingLink, SourceProfile,
};
use studiolo_core::VaultPath;

/// 提交给一次索引重建的 Markdown 笔记。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownDocument {
    path: VaultPath,
    contents: String,
    unavailable: Option<DocumentUnavailable>,
}

impl MarkdownDocument {
    /// 创建一篇输入笔记，不触碰文件系统。
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

    /// 返回逻辑 Vault 路径。
    #[must_use]
    pub fn path(&self) -> &VaultPath {
        &self.path
    }

    /// 返回原始 Markdown 文本。
    #[must_use]
    pub fn contents(&self) -> &str {
        &self.contents
    }
}

/// 一次完成的内存重建附带的计数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexGeneration {
    /// 进程内单调递增的代次号：每次发布加一，到上限后饱和。
    pub generation: u64,
    /// 已索引的笔记数（正文缺席的降级项也计入）。
    pub documents: usize,
    /// 解析出的 wikilink 数。
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

/// 公开索引接缝返回的错误。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    /// 同一逻辑路径在一次重建中出现了两次。
    #[error("duplicate document path: {0}")]
    DuplicatePath(VaultPath),
    /// 查询的路径不在当前代次中。
    #[error("document is not indexed: {0}")]
    NotIndexed(VaultPath),
}

/// 一篇 Markdown 笔记解析后的派生视图。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedDocument {
    /// 逻辑源路径。
    pub path: VaultPath,
    /// 展示标题：依次取 frontmatter 标题、第一个一级标题、文件名。
    pub title: DocumentTitle,
    /// 读侧正规化后的 frontmatter，保留原始拼写。
    pub frontmatter: NormalizedFrontmatter,
    /// 按源顺序排列的扁平标题列表。
    pub headings: Vec<Heading>,
    /// 嵌套的标题大纲。
    pub heading_tree: Vec<HeadingNode>,
    /// 从 Markdown 解析出的 wikilink，附各自在本代次内的解析结果。
    pub outgoing_links: Vec<OutgoingLink>,
    /// 参考性的来源与密级事实。
    pub source_profile: SourceProfile,
    /// 非致命的解析诊断。
    pub warnings: Vec<ParseWarning>,
    /// 本篇正文缺席的原因；`Some` 表示这是降级诊断项而不是完整解析结果。
    pub unavailable: Option<DocumentUnavailable>,
}

/// 展示标题及选中它的规则。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentTitle {
    /// 未经转义的标题文本。
    pub text: String,
    /// 选中该标题所用的规则。
    pub source: TitleSource,
}

/// 选择笔记标题时的有序回退链。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleSource {
    /// 正规化后 frontmatter 的 `title` 文本值。
    Frontmatter,
    /// 第一个一级 Markdown 标题。
    FirstHeading,
    /// 去掉 `.md` 后缀的文件名。
    FileName,
}

/// 非致命的文档解析诊断。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseWarning {
    /// frontmatter 解析或正规化警告。
    Frontmatter(FrontmatterWarning),
}
