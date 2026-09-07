use std::collections::BTreeMap;

use studiolo_core::VaultPath;
use studiolo_vault_index::{
    InMemoryVaultIndex, IndexedDocument, LinkResolution, OutgoingLink, VaultIndex,
};

use crate::{
    SourceLocation,
    navigation_dto::{
        BacklinkView, DocumentNavigation, LinkDestination, NavigationLink, OutlineEntry,
    },
};

const LIST_LIMIT: usize = 200;
const TARGET_LIMIT: usize = 50;
const TEXT_LIMIT: usize = 320;

pub(crate) fn navigation(
    index: &InMemoryVaultIndex,
    path: &VaultPath,
) -> Option<DocumentNavigation> {
    let document = index.document(path).ok()??;
    let incoming = index.backlinks(path).ok()?;
    let mut truncated = document.headings.len() > LIST_LIMIT
        || document.outgoing_links.len() > LIST_LIMIT
        || incoming.len() > LIST_LIMIT;
    let outline = document
        .headings
        .iter()
        .take(LIST_LIMIT)
        .map(|heading| OutlineEntry {
            title: clip(&heading.text, &mut truncated),
            level: heading.level,
            source: source(path, heading.line),
        })
        .collect();
    let outgoing = document
        .outgoing_links
        .iter()
        .take(LIST_LIMIT)
        .map(|link| outgoing_link(index, &document, link, &mut truncated))
        .collect();
    let mut titles = BTreeMap::from([(path.clone(), clip(&document.title.text, &mut truncated))]);
    let backlinks = incoming
        .iter()
        .take(LIST_LIMIT)
        .filter_map(|link| {
            if !titles.contains_key(&link.from) {
                let source_document = index.document(&link.from).ok()??;
                titles.insert(
                    link.from.clone(),
                    clip(&source_document.title.text, &mut truncated),
                );
            }
            Some(BacklinkView {
                title: titles.get(&link.from)?.clone(),
                source: source(&link.from, link.line),
                context: clip(&link.context, &mut truncated),
            })
        })
        .collect();
    Some(DocumentNavigation {
        outline,
        outgoing,
        backlinks,
        truncated,
    })
}

fn outgoing_link(
    index: &InMemoryVaultIndex,
    from: &IndexedDocument,
    link: &OutgoingLink,
    truncated: &mut bool,
) -> NavigationLink {
    let (status, mut targets) = match &link.resolution {
        LinkResolution::Dangling => ("missing", Vec::new()),
        LinkResolution::Resolved { path, .. } => {
            let targets = destinations(
                index,
                from,
                path,
                link.target.heading.as_deref(),
                TARGET_LIMIT + 1,
                truncated,
            );
            let status = match (link.target.heading.is_some(), targets.len()) {
                (true, 0) => "missing_heading",
                (true, 2..) => "ambiguous_heading",
                (_, 0) => "missing",
                _ => "resolved",
            };
            (status, targets)
        }
        LinkResolution::Ambiguous { candidates } => {
            let mut targets = Vec::new();
            if candidates.len() > TARGET_LIMIT {
                *truncated = true;
            }
            for path in candidates.iter().take(TARGET_LIMIT + 1) {
                let remaining = TARGET_LIMIT + 1 - targets.len();
                if remaining == 0 {
                    break;
                }
                targets.extend(destinations(
                    index,
                    from,
                    path,
                    link.target.heading.as_deref(),
                    remaining,
                    truncated,
                ));
            }
            ("ambiguous", targets)
        }
    };
    if targets.len() > TARGET_LIMIT {
        targets.truncate(TARGET_LIMIT);
        *truncated = true;
    }
    let fallback = match &link.target.heading {
        Some(heading) => format!("{}#{heading}", link.target.target),
        None => link.target.target.clone(),
    };
    NavigationLink {
        label: clip(link.target.alias.as_deref().unwrap_or(&fallback), truncated),
        raw: clip(&link.target.raw, truncated),
        source: source(&from.path, link.line),
        status: status.to_owned(),
        targets,
    }
}

fn destinations(
    index: &InMemoryVaultIndex,
    current: &IndexedDocument,
    path: &VaultPath,
    heading: Option<&str>,
    limit: usize,
    truncated: &mut bool,
) -> Vec<LinkDestination> {
    if path == &current.path {
        return heading_destinations(current, heading, limit, truncated);
    }
    let Some(document) = index.document(path).ok().flatten() else {
        return Vec::new();
    };
    heading_destinations(&document, heading, limit, truncated)
}

fn heading_destinations(
    document: &IndexedDocument,
    heading: Option<&str>,
    limit: usize,
    truncated: &mut bool,
) -> Vec<LinkDestination> {
    let Some(fragment) = heading else {
        return vec![destination(document, 1, None, truncated)];
    };
    let text_match = document
        .headings
        .iter()
        .any(|heading| heading.text == fragment);
    document
        .headings
        .iter()
        .filter(|heading| {
            if text_match {
                heading.text == fragment
            } else {
                heading.anchor == fragment
            }
        })
        .take(limit)
        .map(|heading| destination(document, heading.line, Some(&heading.text), truncated))
        .collect()
}

fn destination(
    document: &IndexedDocument,
    line: u32,
    heading: Option<&str>,
    truncated: &mut bool,
) -> LinkDestination {
    let mut title = clip(&document.title.text, truncated);
    if let Some(heading) = heading {
        title.push_str(" · ");
        title.extend(heading.chars().take(TEXT_LIMIT + 1));
    }
    LinkDestination {
        path: document.path.as_str().to_owned(),
        line,
        title: clip(&title, truncated),
    }
}

fn source(path: &VaultPath, line: u32) -> SourceLocation {
    SourceLocation {
        path: path.as_str().to_owned(),
        line,
    }
}

fn clip(value: &str, truncated: &mut bool) -> String {
    let mut characters = value.chars().take(TEXT_LIMIT + 1).collect::<Vec<_>>();
    if characters.len() > TEXT_LIMIT {
        characters.truncate(TEXT_LIMIT - 1);
        characters.push('…');
        *truncated = true;
    }
    characters.into_iter().collect()
}
