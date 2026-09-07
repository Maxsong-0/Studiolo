use std::collections::{BTreeMap, BTreeSet};

use studiolo_core::VaultPath;

use crate::{FrontmatterValue, IndexedDocument, LinkResolution, ResolutionRule, WikilinkTarget};

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
        && let Some(resolution) = exact_path(documents, &relative, ResolutionRule::ExactRelative)
    {
        return resolution;
    }
    let root_candidate = target.target.trim_start_matches('/');
    if let Some(resolution) = exact_path(documents, root_candidate, ResolutionRule::VaultRootPath) {
        return resolution;
    }
    if target.target.contains('/') {
        return LinkResolution::Dangling;
    }
    let name_key = comparison_key(stem(&target.target));
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
    rule: ResolutionRule,
) -> Option<LinkResolution> {
    let with_extension = if candidate.to_lowercase().ends_with(".md") {
        candidate.to_owned()
    } else {
        format!("{candidate}.md")
    };
    let key = comparison_key(&with_extension);
    let mut candidates: Vec<_> = documents
        .keys()
        .filter(|path| comparison_key(path.as_str()) == key)
        .cloned()
        .collect();
    match candidates.len() {
        0 => None,
        1 => Some(LinkResolution::Resolved {
            path: candidates.remove(0),
            rule,
        }),
        _ => Some(LinkResolution::Ambiguous { candidates }),
    }
}

fn stem(filename: &str) -> &str {
    filename
        .rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("md"))
        .map_or(filename, |(stem, _)| stem)
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
        comparison_key(stem(filename)),
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
            _ => {}
        }
    }
    names
}
