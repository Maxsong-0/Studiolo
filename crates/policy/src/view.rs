//! 供给视图：部署上限（`--allow`）与能力集白名单的交集。
//!
//! 交集取较窄的一方；能力集写得再宽也不产生部署上限之外的路径，部署上限再宽也不给
//! 能力集没列的路径。没有能力集的类型不调用本函数——它是零权限，不是空视图。

use studiolo_core::VaultPath;

/// 返回两组前缀的交集（去重、排序）。
#[must_use]
pub fn intersect_prefixes(allow: &[VaultPath], whitelist: &[VaultPath]) -> Vec<VaultPath> {
    let mut result: Vec<VaultPath> = Vec::new();
    for deployed in allow {
        for wanted in whitelist {
            let narrower =
                if deployed == wanted || wanted.as_str().starts_with(&format!("{deployed}/")) {
                    Some(wanted.clone())
                } else if deployed.as_str().starts_with(&format!("{wanted}/")) {
                    Some(deployed.clone())
                } else {
                    None
                };
            if let Some(prefix) = narrower
                && !result.contains(&prefix)
            {
                result.push(prefix);
            }
        }
    }
    // 去掉被更宽前缀覆盖的子前缀，避免同一目录出现两次。
    let covered = result
        .iter()
        .filter(|candidate| {
            result
                .iter()
                .any(|other| candidate.as_str().starts_with(&format!("{other}/")))
        })
        .cloned()
        .collect::<Vec<_>>();
    result.retain(|prefix| !covered.contains(prefix));
    result.sort();
    result
}
