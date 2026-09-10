use std::collections::BTreeMap;

use studiolo_core::VaultPath;

use crate::IndexedDocument;

pub(crate) use crate::{link_parse::parse_outgoing, link_resolve::resolve};

/// Parsed target inside `[[target#heading|alias]]`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct WikilinkTarget {
    /// Exact text between the square brackets.
    pub raw: String,
    /// Path, title or alias before `#` and `|`.
    pub target: String,
    /// Optional heading fragment.
    pub heading: Option<String>,
    /// Optional display alias.
    pub alias: Option<String>,
    /// Whether the source form was an embed (`![[...]]`).
    pub embed: bool,
}

impl WikilinkTarget {
    /// Parses one complete wikilink or embed token.
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

/// Failure to parse a complete wikilink token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WikilinkParseError {
    /// Opening or closing square brackets were absent.
    #[error("wikilink delimiters are missing")]
    MissingDelimiters,
    /// Both target and heading were empty.
    #[error("wikilink target is empty")]
    Empty,
}

/// Rule that uniquely selected a note.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResolutionRule {
    /// A path relative to the source note matched.
    ExactRelative,
    /// A path relative to the Vault root matched.
    VaultRootPath,
    /// A unique name or alias matched in the nearest ancestor subtree.
    NearestUnique {
        /// Number of parent levels traversed from the source directory.
        ancestor_depth: u16,
    },
}

/// Total result of wikilink resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkResolution {
    /// Exactly one note was selected.
    Resolved {
        /// Selected logical path.
        path: VaultPath,
        /// Rule that selected the path.
        rule: ResolutionRule,
    },
    /// Multiple candidates matched a case-folded exact path or remained at the
    /// Vault-wide scope.
    Ambiguous {
        /// Stable, path-sorted candidate list.
        candidates: Vec<VaultPath>,
    },
    /// No note matched.
    Dangling,
}

/// Wikilink parsed from one indexed note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingLink {
    /// Parsed target.
    pub target: WikilinkTarget,
    /// Resolution in the current index generation.
    pub resolution: LinkResolution,
    /// One-based source line.
    pub line: u32,
    /// One-based Unicode-scalar column.
    pub column: u32,
    /// Original source line for display.
    pub context: String,
}

/// One resolved inbound link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backlink {
    /// Source note.
    pub from: VaultPath,
    /// One-based source line.
    pub line: u32,
    /// One-based source column.
    pub column: u32,
    /// Source line for display.
    pub context: String,
    /// Display alias from the wikilink, if present.
    pub alias: Option<String>,
}

/// One dangling link shown by health checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkIssue {
    /// Source note.
    pub from: VaultPath,
    /// Parsed target.
    pub target: WikilinkTarget,
    /// One-based source line.
    pub line: u32,
}

/// One link whose candidates cannot be selected safely.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmbiguousLink {
    /// Source note.
    pub from: VaultPath,
    /// Parsed target.
    pub target: WikilinkTarget,
    /// Stable candidate list.
    pub candidates: Vec<VaultPath>,
    /// One-based source line.
    pub line: u32,
}

/// Derived health view for all indexed links.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkHealth {
    /// Links with no candidates.
    pub dangling: Vec<LinkIssue>,
    /// Links with more than one candidate.
    pub ambiguous: Vec<AmbiguousLink>,
    /// Count of successful resolutions by rule.
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
