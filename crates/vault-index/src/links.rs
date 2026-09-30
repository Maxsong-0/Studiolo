use std::collections::BTreeMap;

use studiolo_core::VaultPath;

use crate::IndexedDocument;

pub(crate) use crate::{link_parse::parse_outgoing, link_resolve::resolve};

/// 从 `[[target#heading|alias]]` 中解析出的目标。
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct WikilinkTarget {
    /// 双方括号之间的原文。
    pub raw: String,
    /// `#` 与 `|` 之前的路径、标题或别名。
    pub target: String,
    /// 可选的标题片段。
    pub heading: Option<String>,
    /// 可选的显示别名。
    pub alias: Option<String>,
    /// 源写法是否为嵌入（`![[...]]`）。
    pub embed: bool,
}

impl WikilinkTarget {
    /// 解析一个完整的 wikilink 或嵌入记号。
    pub fn parse(value: &str) -> Result<Self, WikilinkParseError> {
        let (embed, value) = if let Some(value) = value.strip_prefix("![[") {
            (true, value)
        } else if let Some(value) = value.strip_prefix("[[") {
            (false, value)
        } else {
            return Err(WikilinkParseError::MissingDelimiters);
        };
        let Some(inner) = value.strip_suffix("]]") else {
            return Err(WikilinkParseError::MissingDelimiters);
        };
        if inner.trim().is_empty() {
            return Err(WikilinkParseError::Empty);
        }
        let (address, alias) = inner
            .split_once('|')
            .map_or((inner, None), |(address, alias)| {
                (address, nonempty(alias.trim()))
            });
        let (target, heading) = address
            .split_once('#')
            .map_or((address, None), |(target, heading)| {
                (target, nonempty(heading.trim()))
            });
        if target.trim().is_empty() && heading.is_none() {
            return Err(WikilinkParseError::Empty);
        }
        Ok(Self {
            raw: inner.to_owned(),
            target: target.trim().to_owned(),
            heading: heading.map(str::to_owned),
            alias: alias.map(str::to_owned),
            embed,
        })
    }
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

/// 完整 wikilink 记号的解析失败。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WikilinkParseError {
    /// 缺少起始或结束的双方括号。
    #[error("wikilink delimiters are missing")]
    MissingDelimiters,
    /// 目标与标题片段都为空。
    #[error("wikilink target is empty")]
    Empty,
}

/// 唯一选中一篇笔记所依据的规则。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResolutionRule {
    /// 相对源笔记的路径精确命中；纯锚点链接（`[[#小节]]`）指向源笔记自身，也记为此规则。
    ExactRelative,
    /// 相对 Vault 根的路径精确命中。
    VaultRootPath,
    /// 在最近的祖先子树中按名称、标题或别名唯一命中。
    NearestUnique {
        /// 从源目录向上走过的父级层数。
        ancestor_depth: u16,
    },
}

/// wikilink 解析的完整结果：必定落在以下三者之一。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkResolution {
    /// 恰好选中一篇笔记。
    Resolved {
        /// 选中的逻辑路径。
        path: VaultPath,
        /// 选中该路径所依据的规则。
        rule: ResolutionRule,
    },
    /// 多个候选在大小写折叠后精确命中同一路径，或就近查找在最近一层就不唯一（此时给出
    /// Vault 全域按名称、标题或别名命中的候选）。
    Ambiguous {
        /// 按路径排序的稳定候选列表。
        candidates: Vec<VaultPath>,
    },
    /// 没有任何笔记命中。
    Dangling,
}

/// 从一篇已索引笔记中解析出的 wikilink。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingLink {
    /// 解析出的目标。
    pub target: WikilinkTarget,
    /// 在当前索引代次中的解析结果。
    pub resolution: LinkResolution,
    /// 源文件行号（从 1 起）。
    pub line: u32,
    /// 以 Unicode 标量计的列号（从 1 起）。
    pub column: u32,
    /// 供展示的源行窗口：最多 320 个 Unicode 标量（含省略号），短行逐字保留。
    pub context: String,
}

/// 一条已解析的入链。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backlink {
    /// 来源笔记。
    pub from: VaultPath,
    /// 来源行号（从 1 起）。
    pub line: u32,
    /// 来源列号（从 1 起）。
    pub column: u32,
    /// 供展示的来源行窗口：最多 320 个 Unicode 标量（含省略号），短行逐字保留。
    pub context: String,
    /// wikilink 上的显示别名（如有）。
    pub alias: Option<String>,
}

/// 健康检查中列出的一条悬空链接。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkIssue {
    /// 来源笔记。
    pub from: VaultPath,
    /// 解析出的目标。
    pub target: WikilinkTarget,
    /// 来源行号（从 1 起）。
    pub line: u32,
}

/// 一条无法安全选定候选的链接。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmbiguousLink {
    /// 来源笔记。
    pub from: VaultPath,
    /// 解析出的目标。
    pub target: WikilinkTarget,
    /// 稳定排序的候选列表。
    pub candidates: Vec<VaultPath>,
    /// 来源行号（从 1 起）。
    pub line: u32,
}

/// 覆盖全部已索引链接的派生健康视图。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkHealth {
    /// 没有任何候选的链接。
    pub dangling: Vec<LinkIssue>,
    /// 候选多于一个的链接。
    pub ambiguous: Vec<AmbiguousLink>,
    /// 按规则统计的成功解析次数。
    pub rule_histogram: BTreeMap<ResolutionRule, usize>,
}

pub(crate) fn derive_links(
    documents: &BTreeMap<VaultPath, IndexedDocument>,
) -> (BTreeMap<VaultPath, Vec<Backlink>>, LinkHealth) {
    let mut backlinks: BTreeMap<VaultPath, Vec<Backlink>> = BTreeMap::new();
    let mut health = LinkHealth::default();
    for (from, document) in documents {
        for link in &document.outgoing_links {
            match &link.resolution {
                LinkResolution::Resolved { path, rule } => {
                    backlinks.entry(path.clone()).or_default().push(Backlink {
                        from: from.clone(),
                        line: link.line,
                        column: link.column,
                        context: link.context.clone(),
                        alias: link.target.alias.clone(),
                    });
                    *health.rule_histogram.entry(*rule).or_default() += 1;
                }
                LinkResolution::Ambiguous { candidates } => {
                    health.ambiguous.push(AmbiguousLink {
                        from: from.clone(),
                        target: link.target.clone(),
                        candidates: candidates.clone(),
                        line: link.line,
                    });
                }
                LinkResolution::Dangling => health.dangling.push(LinkIssue {
                    from: from.clone(),
                    target: link.target.clone(),
                    line: link.line,
                }),
            }
        }
    }
    for values in backlinks.values_mut() {
        values.sort_by(|left, right| {
            (&left.from, left.line, left.column).cmp(&(&right.from, right.line, right.column))
        });
    }
    (backlinks, health)
}
