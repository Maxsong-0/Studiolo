use std::collections::{BTreeMap, BTreeSet};

use studiolo_core::VaultPath;

use crate::{FrontmatterValue, IndexedDocument};

/// Parsed target inside `[[target#heading|alias]]`.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Multiple candidates remained at the Vault-wide scope.
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

pub(crate) fn parse_outgoing(body: &str, first_line: u32) -> Vec<OutgoingLink> {
    let mut links = Vec::new();
    let mut in_fence = false;
    for (offset, line) in body.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut cursor = 0;
        while let Some(relative) = line[cursor..].find("[[") {
            let brackets = cursor + relative;
            if line[..brackets]
                .chars()
                .filter(|character| *character == '`')
                .count()
                % 2
                == 1
            {
                cursor = brackets + 2;
                continue;
            }
            let Some(close_relative) = line[brackets + 2..].find("]]") else {
                break;
            };
            let close = brackets + 2 + close_relative + 2;
            let token_start = if line[..brackets].ends_with('!') {
                brackets - 1
            } else {
                brackets
            };
            if let Ok(target) = WikilinkTarget::parse(&line[token_start..close]) {
                links.push(OutgoingLink {
                    target,
                    resolution: LinkResolution::Dangling,
                    line: first_line + offset as u32,
                    column: line[..token_start].chars().count() as u32 + 1,
                    context: line.to_owned(),
                });
            }
            cursor = close;
        }
    }
    links
}

pub(crate) fn resolve(
    documents: &BTreeMap<VaultPath, IndexedDocument>,
    from: &VaultPath,
    target: &WikilinkTarget,
) -> LinkResolution {
    if target.target.is_empty() && target.heading.is_some() {
        return LinkResolution::Resolved {
            path: from.clone(),
            rule: ResolutionRule::ExactRelative,
        };
    }

    if !target.target.starts_with('/')
        && let Some(relative) = relative_candidate(from, &target.target)
        && let Some(path) = exact_path(documents, &relative)
    {
        return LinkResolution::Resolved {
            path,
            rule: ResolutionRule::ExactRelative,
        };
    }

    let root_candidate = target.target.trim_start_matches('/');
    if let Some(path) = exact_path(documents, root_candidate) {
        return LinkResolution::Resolved {
            path,
            rule: ResolutionRule::VaultRootPath,
        };
    }

    if target.target.contains('/') {
        return LinkResolution::Dangling;
    }

    let name = target
        .target
        .rsplit('/')
        .next()
        .unwrap_or(&target.target)
        .strip_suffix(".md")
        .unwrap_or_else(|| target.target.rsplit('/').next().unwrap_or(&target.target));
    let name_key = comparison_key(name);
    let mut ancestor = parent(from.as_str()).unwrap_or_default().to_owned();
    let mut depth = 0_u16;
    loop {
        let mut candidates: Vec<_> = documents
            .iter()
            .filter(|(path, document)| {
                is_in_subtree(path.as_str(), &ancestor)
                    && document_names(document).contains(&name_key)
            })
            .map(|(path, _)| path.clone())
            .collect();
        candidates.sort();
        candidates.dedup();
        if candidates.len() == 1 {
            return LinkResolution::Resolved {
                path: candidates.remove(0),
                rule: ResolutionRule::NearestUnique {
                    ancestor_depth: depth,
                },
            };
        }
        if ancestor.is_empty() {
            return if candidates.is_empty() {
                LinkResolution::Dangling
            } else {
                LinkResolution::Ambiguous { candidates }
            };
        }
        ancestor = parent(&ancestor).unwrap_or_default().to_owned();
        depth = depth.saturating_add(1);
    }
}

fn relative_candidate(from: &VaultPath, target: &str) -> Option<String> {
    let mut components: Vec<&str> = from.as_str().split('/').collect();
    components.pop();
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            value => components.push(value),
        }
    }
    Some(components.join("/"))
}

fn exact_path(
    documents: &BTreeMap<VaultPath, IndexedDocument>,
    candidate: &str,
) -> Option<VaultPath> {
    let with_extension = if candidate.ends_with(".md") {
        candidate.to_owned()
    } else {
        format!("{candidate}.md")
    };
    let key = comparison_key(&with_extension);
    documents
        .keys()
        .find(|path| comparison_key(path.as_str()) == key)
        .cloned()
}

fn comparison_key(value: &str) -> String {
    value.trim().to_lowercase()
}

fn parent(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(parent, _)| parent)
}

fn is_in_subtree(path: &str, ancestor: &str) -> bool {
    ancestor.is_empty()
        || path == ancestor
        || path
            .strip_prefix(ancestor)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn document_names(document: &IndexedDocument) -> BTreeSet<String> {
    let filename = document
        .path
        .as_str()
        .rsplit('/')
        .next()
        .unwrap_or(document.path.as_str());
    let mut names = BTreeSet::from([
        comparison_key(filename.strip_suffix(".md").unwrap_or(filename)),
        comparison_key(&document.title.text),
    ]);
    if let Some(value) = document.frontmatter.get("aliases") {
        match value {
            FrontmatterValue::Text(alias) => {
                names.insert(comparison_key(alias));
            }
            FrontmatterValue::List(aliases) => {
                for alias in aliases {
                    if let FrontmatterValue::Text(alias) = alias {
                        names.insert(comparison_key(alias));
                    }
                }
            }
            FrontmatterValue::Null
            | FrontmatterValue::Bool(_)
            | FrontmatterValue::Number(_)
            | FrontmatterValue::Mapping(_)
            | FrontmatterValue::Status(_) => {}
        }
    }
    names
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
