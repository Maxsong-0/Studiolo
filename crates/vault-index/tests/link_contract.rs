//! Public-seam contract tests for wikilink resolution and backlinks.

use studiolo_core::VaultPath;
use studiolo_vault_index::{
    InMemoryVaultIndex, LinkResolution, MarkdownDocument, ResolutionRule, VaultIndex,
    WikilinkTarget,
};

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("fixture path is valid")
}

fn note(path_text: &str, contents: &str) -> MarkdownDocument {
    MarkdownDocument::new(path(path_text), contents)
}

#[test]
fn public_index_seam_resolves_nearest_links_and_builds_only_resolved_backlinks() {
    let journal = path("项目/甲/02-日志/今天.md");
    let alpha_tasks = path("项目/甲/04-协调/TASKS.md");
    let beta_tasks = path("项目/乙/04-协调/TASKS.md");
    let local = path("项目/甲/02-日志/同目录.md");
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            note(
                journal.as_str(),
                "[[TASKS#本周|协调板]] [[同目录]] [[项目/乙/04-协调/TASKS]] [[不存在]]",
            ),
            note(
                alpha_tasks.as_str(),
                "---\naliases: [协调中心]\n---\n# 甲项目任务\n## 本周\n",
            ),
            note(beta_tasks.as_str(), "# 乙项目任务"),
            note(local.as_str(), "# 本地页"),
            note("同目录.md", "# 根页"),
            note("入口.md", "[[TASKS]]"),
        ])
        .expect("index rebuild succeeds");

    let nearest = index
        .resolve_link(
            &journal,
            &WikilinkTarget::parse("[[TASKS#本周|协调板]]").expect("valid wikilink"),
        )
        .expect("resolution succeeds");
    assert_eq!(
        nearest,
        LinkResolution::Resolved {
            path: alpha_tasks.clone(),
            rule: ResolutionRule::NearestUnique { ancestor_depth: 1 },
        }
    );

    let alias = index
        .resolve_link(
            &journal,
            &WikilinkTarget::parse("[[协调中心]]").expect("valid wikilink"),
        )
        .expect("resolution succeeds");
    assert_eq!(
        alias,
        LinkResolution::Resolved {
            path: alpha_tasks.clone(),
            rule: ResolutionRule::NearestUnique { ancestor_depth: 1 },
        }
    );

    let exact = index
        .resolve_link(
            &journal,
            &WikilinkTarget::parse("[[同目录]]").expect("valid wikilink"),
        )
        .expect("resolution succeeds");
    assert_eq!(
        exact,
        LinkResolution::Resolved {
            path: local,
            rule: ResolutionRule::ExactRelative,
        }
    );

    let root = index
        .resolve_link(
            &journal,
            &WikilinkTarget::parse("[[项目/乙/04-协调/TASKS]]").expect("valid wikilink"),
        )
        .expect("resolution succeeds");
    assert_eq!(
        root,
        LinkResolution::Resolved {
            path: beta_tasks.clone(),
            rule: ResolutionRule::VaultRootPath,
        }
    );

    let missing_qualified_path = index
        .resolve_link(
            &journal,
            &WikilinkTarget::parse("[[项目/丙/04-协调/TASKS]]").expect("valid wikilink"),
        )
        .expect("resolution succeeds");
    assert_eq!(missing_qualified_path, LinkResolution::Dangling);

    let ambiguous = index
        .resolve_link(
            &path("入口.md"),
            &WikilinkTarget::parse("[[TASKS]]").expect("valid wikilink"),
        )
        .expect("resolution succeeds");
    assert_eq!(
        ambiguous,
        LinkResolution::Ambiguous {
            candidates: vec![beta_tasks, alpha_tasks.clone()],
        }
    );

    let backlinks = index
        .backlinks(&alpha_tasks)
        .expect("backlink query succeeds");
    assert_eq!(backlinks.len(), 1);
    assert_eq!(backlinks[0].from, journal);
    assert_eq!(backlinks[0].alias.as_deref(), Some("协调板"));

    let health = index.link_health().expect("health query succeeds");
    assert_eq!(health.dangling.len(), 1);
    assert_eq!(health.ambiguous.len(), 1);
}
