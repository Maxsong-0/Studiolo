//! 供给视图 = 部署上限 ∩ 能力集白名单，取较窄一方；不产生任何一方之外的路径。

use studiolo_core::VaultPath;
use studiolo_policy::view::intersect_prefixes;

fn paths(values: &[&str]) -> Vec<VaultPath> {
    values
        .iter()
        .map(|value| VaultPath::parse(value).unwrap())
        .collect()
}

#[test]
fn intersection_takes_the_narrower_prefix_and_drops_uncovered_entries() {
    let allow = paths(&["项目/Studiolo", "_规则", "项目/Semiment"]);
    assert_eq!(
        intersect_prefixes(&allow, &paths(&["项目/Studiolo", "School"])),
        paths(&["项目/Studiolo"])
    );
    // 能力集比部署上限窄：取能力集。
    assert_eq!(
        intersect_prefixes(&allow, &paths(&["项目/Studiolo/03-记忆"])),
        paths(&["项目/Studiolo/03-记忆"])
    );
    // 能力集比部署上限宽：取部署上限，且不给上限之外的兄弟目录。
    assert_eq!(
        intersect_prefixes(&allow, &paths(&["项目"])),
        paths(&["项目/Semiment", "项目/Studiolo"])
    );
    // 同名前缀的伪匹配（`项目/Studiolo2`）不算覆盖。
    assert!(intersect_prefixes(&allow, &paths(&["项目/Studiolo2"])).is_empty());
    assert!(intersect_prefixes(&allow, &[]).is_empty());
    assert!(intersect_prefixes(&[], &paths(&["项目/Studiolo"])).is_empty());
    // 子前缀被更宽的前缀覆盖时去重。
    assert_eq!(
        intersect_prefixes(
            &allow,
            &paths(&["项目/Studiolo/03-记忆", "项目/Studiolo", "_规则"])
        ),
        paths(&["_规则", "项目/Studiolo"])
    );
}
