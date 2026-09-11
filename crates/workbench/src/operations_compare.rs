//! 仅凭完整、同代观测判断双向漂移；缺失观测永远不是空目录。

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    DirectoryDrift, DirectoryObservation, ProjectOperations, operations_parse, tree_validate,
};

impl ProjectOperations {
    /// 将声明与明确展开父层的完整观测比较；无绑定或观测无效时保留未验证状态。
    #[must_use]
    pub fn compare(&self, observations: &[DirectoryObservation]) -> Vec<DirectoryDrift> {
        let mut result = BTreeMap::new();
        let observations = validated(self, observations).unwrap_or_default();
        let contradictions = contradiction_roots(&observations);
        for directory in &self.directories {
            let status = evidence(&directory.path, &observations, &contradictions);
            result.insert(
                directory.path.clone(),
                DirectoryDrift {
                    path: directory.path.clone(),
                    status: status.to_owned(),
                    source: directory.source.clone(),
                },
            );
        }
        for observation in observations
            .values()
            .filter(|observation| observation.status == "ready")
        {
            for path in &observation.children {
                result.entry(path.clone()).or_insert_with(|| {
                    let status = if evidence(path, &observations, &contradictions) == "present" {
                        "undocumented"
                    } else {
                        "unverified"
                    };
                    DirectoryDrift {
                        path: path.clone(),
                        status: status.to_owned(),
                        source: None,
                    }
                });
            }
        }
        result.into_values().collect()
    }
}

fn validated<'a>(
    declaration: &ProjectOperations,
    observations: &'a [DirectoryObservation],
) -> Option<BTreeMap<&'a str, &'a DirectoryObservation>> {
    if declaration.declaration_status != "ready" || observations.len() > 64 {
        return None;
    }
    let mut result = BTreeMap::new();
    for observation in observations {
        if (!observation.path.is_empty() && !operations_parse::repository_path(&observation.path))
            || !declaration.comparison_parents.contains(&observation.path)
            || result
                .insert(observation.path.as_str(), observation)
                .is_some()
            || !matches!(
                observation.status.as_str(),
                "ready" | "missing" | "unavailable"
            )
            || (observation.status != "ready" && !observation.children.is_empty())
        {
            return None;
        }
        let mut unique = BTreeSet::new();
        for child in &observation.children {
            if !operations_parse::repository_path(child)
                || tree_validate::parent(child) != observation.path
                || !unique.insert(child.to_lowercase())
            {
                return None;
            }
        }
    }
    Some(result)
}

/// 深处存在可证明其祖先存在；完整父层或 missing 与此冲突时，整条分支不可验证。
fn contradiction_roots(observations: &BTreeMap<&str, &DirectoryObservation>) -> BTreeSet<String> {
    let mut present = BTreeSet::new();
    for observation in observations
        .values()
        .filter(|observation| observation.status == "ready")
    {
        for path in std::iter::once(&observation.path).chain(&observation.children) {
            let mut ancestor = path.as_str();
            loop {
                present.insert(ancestor.to_owned());
                if ancestor.is_empty() {
                    break;
                }
                ancestor = tree_validate::parent(ancestor);
            }
        }
    }
    present
        .into_iter()
        .filter(|path| {
            if observations
                .get(path.as_str())
                .is_some_and(|observation| observation.status == "missing")
            {
                return true;
            }
            if path.is_empty() {
                return false;
            }
            observations
                .get(tree_validate::parent(path))
                .is_some_and(|parent| {
                    parent.status == "missing"
                        || (parent.status == "ready" && !parent.children.contains(path))
                })
        })
        .collect()
}

fn evidence(
    path: &str,
    observations: &BTreeMap<&str, &DirectoryObservation>,
    contradictions: &BTreeSet<String>,
) -> &'static str {
    if contradictions.contains("") {
        return "unverified";
    }
    let mut present = false;
    let mut missing = false;
    let mut ancestor = path;
    loop {
        if contradictions.contains(ancestor) {
            return "unverified";
        }
        if let Some(observation) = observations.get(ancestor) {
            match observation.status.as_str() {
                "ready" if ancestor == path => present = true,
                "missing" => missing = true,
                _ => {}
            }
        }
        let parent = tree_validate::parent(ancestor);
        if let Some(observation) = observations.get(parent) {
            if observation.status == "ready" {
                if observation.children.iter().any(|child| child == ancestor) {
                    present |= ancestor == path;
                } else {
                    missing = true;
                }
            } else if observation.status == "missing" {
                missing = true;
            }
        }
        if parent.is_empty() {
            break;
        }
        ancestor = parent;
    }
    match (present, missing) {
        (true, false) => "present",
        (false, true) => "missing",
        _ => "unverified",
    }
}
