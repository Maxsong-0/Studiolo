//! 单条 wikilink 的解析规则。候选一律来自 [`LinkIndex`]，解析代价只与候选数和源文档
//! 目录层数有关，不随库规模增长。

use studiolo_core::VaultPath;

use crate::{
    LinkResolution, ResolutionRule, WikilinkTarget,
    link_index::{LinkIndex, comparison_key, stem},
};

/// 一次解析实际访问的候选数与向上走过的目录层数；供规模证据核对。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResolveProbe {
    pub(crate) candidates_examined: usize,
    pub(crate) ancestor_levels: usize,
}

pub(crate) fn resolve(
    index: &LinkIndex,
    from: &VaultPath,
    target: &WikilinkTarget,
) -> LinkResolution {
    resolve_probed(index, from, target).0
}

pub(crate) fn resolve_probed(
    index: &LinkIndex,
    from: &VaultPath,
    target: &WikilinkTarget,
) -> (LinkResolution, ResolveProbe) {
    let mut probe = ResolveProbe::default();
    // 纯锚点链接（`[[#小节]]`）指向源文档本身，不参与任何候选查找。
    if target.target.is_empty() && target.heading.is_some() {
        return (
            LinkResolution::Resolved {
                path: from.clone(),
                rule: ResolutionRule::ExactRelative,
            },
            probe,
        );
    }
    if !target.target.starts_with('/')
        && let Some(relative) = relative_candidate(from, &target.target)
        && let Some(resolution) =
            exact_path(index, &relative, ResolutionRule::ExactRelative, &mut probe)
    {
        return (resolution, probe);
    }
    let root_candidate = target.target.trim_start_matches('/');
    if let Some(resolution) = exact_path(
        index,
        root_candidate,
        ResolutionRule::VaultRootPath,
        &mut probe,
    ) {
        return (resolution, probe);
    }
    // 带目录的目标只认精确路径；不降级成就近同名匹配，免得跨目录猜错页面。
    if target.target.contains('/') {
        return (LinkResolution::Dangling, probe);
    }
    let resolution = nearest(
        index,
        from,
        &comparison_key(stem(&target.target)),
        &mut probe,
    );
    (resolution, probe)
}

/// 就近唯一：从源目录逐级向根走，第一个非空候选层若恰好唯一即命中。
///
/// 候选集合随层级单调增大，所以只需算出每个候选首次落入的层级：最小层级上唯一即
/// `NearestUnique`，最小层级上已有多个则各层都不可能唯一，直接按 Vault 全域给歧义候选。
fn nearest(
    index: &LinkIndex,
    from: &VaultPath,
    name_key: &str,
    probe: &mut ResolveProbe,
) -> LinkResolution {
    let candidates = index.names_for(name_key);
    // 累加而不是覆盖：精确路径那两步已经记过自己的候选数，探针要反映整条解析路径的总访问量。
    probe.candidates_examined = probe.candidates_examined.saturating_add(candidates.len());
    if candidates.is_empty() {
        return LinkResolution::Dangling;
    }
    let ancestors = ancestor_chain(from);
    probe.ancestor_levels = ancestors.len();
    let last_level = ancestors.len().saturating_sub(1);
    let mut best_level = usize::MAX;
    let mut best: Option<&VaultPath> = None;
    let mut best_count = 0_usize;
    for candidate in candidates {
        let level = ancestors
            .iter()
            .position(|ancestor| is_in_subtree(candidate.as_str(), ancestor))
            .unwrap_or(last_level);
        if level < best_level {
            best_level = level;
            best = Some(candidate);
            best_count = 1;
        } else if level == best_level {
            best_count += 1;
        }
    }
    match (best_count, best) {
        (1, Some(path)) => LinkResolution::Resolved {
            path: path.clone(),
            rule: ResolutionRule::NearestUnique {
                ancestor_depth: u16::try_from(best_level).unwrap_or(u16::MAX),
            },
        },
        _ => LinkResolution::Ambiguous {
            candidates: candidates.to_vec(),
        },
    }
}

/// 源文档所在目录起、逐级向上直到 Vault 根（空串）的祖先序列。
fn ancestor_chain(from: &VaultPath) -> Vec<&str> {
    let mut chain = vec![parent(from.as_str()).unwrap_or_default()];
    while let Some(last) = chain.last().copied()
        && !last.is_empty()
    {
        chain.push(parent(last).unwrap_or_default());
    }
    chain
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
    index: &LinkIndex,
    candidate: &str,
    rule: ResolutionRule,
    probe: &mut ResolveProbe,
) -> Option<LinkResolution> {
    let with_extension = if candidate.to_lowercase().ends_with(".md") {
        candidate.to_owned()
    } else {
        format!("{candidate}.md")
    };
    let candidates = index.paths_for(&comparison_key(&with_extension));
    probe.candidates_examined = probe.candidates_examined.saturating_add(candidates.len());
    match candidates {
        [] => None,
        [single] => Some(LinkResolution::Resolved {
            path: single.clone(),
            rule,
        }),
        several => Some(LinkResolution::Ambiguous {
            candidates: several.to_vec(),
        }),
    }
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
