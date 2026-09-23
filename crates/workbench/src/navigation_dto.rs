use serde::Serialize;

use crate::SourceLocation;

/// 某个不可变代次中一篇文档的导航视图，各项都链接到源位置。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DocumentNavigation {
    /// 按源顺序排列的扁平大纲，至多 200 条。
    pub outline: Vec<OutlineEntry>,
    /// 按源顺序排列的已解析链接，至多 200 条。
    pub outgoing: Vec<NavigationLink>,
    /// 来自整个 Vault、指向本页的已解析入链，至多 200 条。
    pub backlinks: Vec<BacklinkView>,
    /// 某个列表、候选列表或展示字符串达到了展示上限。
    pub truncated: bool,
}

/// 源文档中的一个标题。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct OutlineEntry {
    /// 纯标题文本，至多 320 个 Unicode 标量。
    pub title: String,
    /// Markdown 标题级别，1 到 6。
    pub level: u8,
    /// 标题所在的确切源行。
    pub source: SourceLocation,
}

/// 一条已解析的 wikilink，附显式、不做猜测的解析结果。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NavigationLink {
    /// 显示别名；没有别名时为目标加可选的标题片段。
    pub label: String,
    /// `[[` 与 `]]` 之间的原文，展示时截断到至多 320 个标量。
    pub raw: String,
    /// 链接所在的确切行。
    pub source: SourceLocation,
    /// `resolved`、`ambiguous`、`missing`、`missing_heading` 或 `ambiguous_heading`。
    pub status: String,
    /// 只含真实存在的目的地；至多 50 条，逻辑路径完整。
    pub targets: Vec<LinkDestination>,
}

/// 一个真实、可选中的笔记或标题目的地。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LinkDestination {
    /// 完整的逻辑 Vault 路径；绝不截断，也不转换成宿主路径。
    pub path: String,
    /// 目标行号（从 1 起）；只指向页面的链接为 1。
    pub line: u32,
    /// 目标文档的展示标题，带可选的标题片段。
    pub title: String,
}

/// 一条真实的页面入链，保留其源上下文。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct BacklinkView {
    /// 来源文档的展示标题。
    pub title: String,
    /// 包含该入链的确切源行。
    pub source: SourceLocation,
    /// 源行原文，展示时截断到至多 320 个 Unicode 标量。
    pub context: String,
}
