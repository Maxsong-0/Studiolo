//! Studiolo Vault 中 Markdown 笔记的只读、可重建视图。
//!
//! 笔记文本一律由调用方显式提供；本 crate 从不自行打开或写入 canonical Vault。

#![forbid(unsafe_code)]

mod build;
mod cache;
mod frontmatter;
mod frontmatter_types;
mod frontmatter_yaml;
mod link_index;
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
    DocumentTitle, DocumentUnavailable, IndexDiagnostic, IndexError, IndexGeneration,
    IndexedDocument, MarkdownDocument, ParseWarning, TitleSource,
};

/// 只读派生 Vault 索引的公开查询接缝。
pub trait VaultIndex: Send + Sync {
    /// 用给定笔记整体替换当前派生代次。
    ///
    /// 只改动内存中的派生数据，不做任何 Vault I/O。
    fn rebuild(&mut self, documents: &[MarkdownDocument]) -> Result<IndexGeneration, IndexError>;

    /// 返回一篇已解析的笔记；未被索引时返回 `None`。
    fn document(&self, path: &VaultPath) -> Result<Option<IndexedDocument>, IndexError>;

    /// 返回一篇笔记的嵌套页面大纲。
    fn heading_tree(&self, path: &VaultPath) -> Result<Vec<HeadingNode>, IndexError>;

    /// 解析一条 wikilink：先按精确路径规则匹配，再做就近优先查找。
    fn resolve_link(
        &self,
        from: &VaultPath,
        target: &WikilinkTarget,
    ) -> Result<LinkResolution, IndexError>;

    /// 返回指向 `to` 的全部已解析链接。
    fn backlinks(&self, to: &VaultPath) -> Result<Vec<Backlink>, IndexError>;

    /// 返回全部悬空链接与确实存在歧义的链接。
    fn link_health(&self) -> Result<LinkHealth, IndexError>;

    /// 返回一篇笔记的参考性来源事实。
    ///
    /// 结果刻意不是 `DataLabels`，不得用于授权判定。
    fn source_profile(&self, path: &VaultPath) -> Result<SourceProfile, IndexError>;

    /// 返回当前代次的计数。
    fn generation(&self) -> IndexGeneration;

    /// 返回本代次全部降级诊断，按逻辑路径排序。
    ///
    /// 符号链接与超大笔记只在这里出现，不会让 rebuild 失败；结果只含逻辑路径。
    fn diagnostics(&self) -> Vec<IndexDiagnostic>;
}
/// [`VaultIndex`] 的 M0 内存适配器。
#[derive(Debug, Default)]
pub struct InMemoryVaultIndex {
    documents: BTreeMap<VaultPath, IndexedDocument>,
    backlinks: BTreeMap<VaultPath, Vec<Backlink>>,
    link_health: LinkHealth,
    generation: IndexGeneration,
    diagnostics: Vec<IndexDiagnostic>,
    pub(crate) link_index: link_index::LinkIndex,
}

impl InMemoryVaultIndex {
    /// 创建一个空索引。
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
                parse::ParsedDocument::parse(input)
                    .finish(input.path(), input.unavailable_reason()),
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
        Ok(links::resolve(&self.link_index, from, target))
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

    fn diagnostics(&self) -> Vec<IndexDiagnostic> {
        self.diagnostics.clone()
    }
}
