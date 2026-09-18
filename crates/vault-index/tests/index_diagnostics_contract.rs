//! 降级诊断的公开契约：符号链接与超大笔记只降级成可查询结论，既不让 rebuild 失败，
//! 也不把 Vault 之外的物理信息带出来。

use studiolo_core::VaultPath;
use studiolo_vault_index::{
    DocumentUnavailable, InMemoryVaultIndex, IndexDiagnostic, LinkResolution, MarkdownDocument,
    ResolutionRule, VaultIndex,
};

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("fixture path is valid")
}

fn note(path_text: &str, contents: &str) -> MarkdownDocument {
    MarkdownDocument::new(path(path_text), contents)
}

fn degraded(path_text: &str, reason: DocumentUnavailable) -> MarkdownDocument {
    MarkdownDocument::unavailable(path(path_text), reason)
}

/// 调用方持有的物理信息样本；任何一段都不许出现在诊断里。
const PHYSICAL_LEAKS: [&str; 6] = [
    "/Users/",
    "/Volumes/",
    "/private/tmp/",
    "C:\\",
    "os error",
    "No such file or directory",
];

#[test]
fn symlinked_and_oversized_notes_degrade_to_diagnostics_instead_of_failing_the_whole_rebuild() {
    let mut index = InMemoryVaultIndex::new();

    let generation = index
        .rebuild(&[
            note("项目/甲/02-日志/今天.md", "# 今天\n见 [[TASKS]]"),
            note("项目/甲/04-协调/TASKS.md", "# 甲任务"),
            degraded("项目/甲/外链笔记.md", DocumentUnavailable::Symlink),
            degraded("项目/甲/巨大附录.md", DocumentUnavailable::TooLarge),
            degraded("项目/甲/乱码.md", DocumentUnavailable::Unreadable),
        ])
        .expect("降级笔记不应让整批构建失败");

    assert_eq!(generation.documents, 5, "降级笔记仍然是被索引的逻辑路径");
    assert_eq!(
        index.diagnostics(),
        vec![
            IndexDiagnostic {
                path: path("项目/甲/乱码.md"),
                reason: DocumentUnavailable::Unreadable,
            },
            IndexDiagnostic {
                path: path("项目/甲/外链笔记.md"),
                reason: DocumentUnavailable::Symlink,
            },
            IndexDiagnostic {
                path: path("项目/甲/巨大附录.md"),
                reason: DocumentUnavailable::TooLarge,
            },
        ],
        "诊断必须按逻辑路径稳定排序，且只含降级项"
    );
}

#[test]
fn a_rendered_diagnostic_carries_only_the_logical_path_and_never_a_physical_prefix() {
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[degraded(
            "项目/甲/外链笔记.md",
            DocumentUnavailable::Symlink,
        )])
        .expect("rebuild succeeds");

    let diagnostics = index.diagnostics();
    let diagnostic = diagnostics.first().expect("正好一条诊断");
    assert_eq!(diagnostic.path.as_str(), "项目/甲/外链笔记.md");

    // 渲染文案与 Debug 输出都是会被端到端带给用户的形态，两者都要查。
    for rendered in [diagnostic.to_string(), format!("{diagnostic:?}")] {
        for leak in PHYSICAL_LEAKS {
            assert!(
                !rendered.contains(leak),
                "诊断文案泄露了物理信息 {leak}：{rendered}"
            );
        }
    }
    assert_eq!(
        diagnostic.to_string(),
        "项目/甲/外链笔记.md：路径上存在符号链接或特殊文件，正文未索引"
    );
}

#[test]
fn a_degraded_note_leaves_healthy_notes_parsing_linking_and_backlinking_untouched() {
    let journal = path("项目/甲/02-日志/今天.md");
    let tasks = path("项目/甲/04-协调/TASKS.md");
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            note(journal.as_str(), "# 今天\n\n## 小节\n见 [[TASKS|协调板]]"),
            note(tasks.as_str(), "# 甲任务"),
            degraded("项目/甲/巨大附录.md", DocumentUnavailable::TooLarge),
        ])
        .expect("rebuild succeeds");

    let healthy = index
        .document(&journal)
        .expect("query succeeds")
        .expect("正常笔记仍然可查");
    assert_eq!(healthy.unavailable, None);
    assert_eq!(healthy.headings.len(), 2);
    assert_eq!(healthy.outgoing_links.len(), 1);

    let backlinks = index.backlinks(&tasks).expect("backlink query succeeds");
    assert_eq!(backlinks.len(), 1);
    assert_eq!(backlinks[0].from, journal);

    let health = index.link_health().expect("health query succeeds");
    assert!(
        health.dangling.is_empty() && health.ambiguous.is_empty(),
        "降级笔记不得凭空制造悬空或歧义链接"
    );
}

#[test]
fn a_degraded_note_yields_no_headings_links_or_frontmatter_of_its_own() {
    let oversized = path("项目/甲/巨大附录.md");
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[degraded(oversized.as_str(), DocumentUnavailable::TooLarge)])
        .expect("rebuild succeeds");

    let document = index
        .document(&oversized)
        .expect("query succeeds")
        .expect("降级笔记仍然是索引里的一条");
    assert_eq!(document.unavailable, Some(DocumentUnavailable::TooLarge));
    assert!(document.headings.is_empty(), "正文缺席时不得编造标题");
    assert!(document.outgoing_links.is_empty(), "正文缺席时不得编造出链");
    assert!(document.frontmatter.warnings.is_empty());
    // 标题只能退回文件名：读不到正文就没有 frontmatter 或 H1 可依据。
    assert_eq!(document.title.text, "巨大附录");
    assert_eq!(
        index
            .heading_tree(&oversized)
            .expect("heading query succeeds"),
        Vec::new()
    );
}

#[test]
fn a_degraded_note_keeps_its_logical_name_so_inbound_links_do_not_silently_retarget() {
    let journal = path("项目/甲/02-日志/今天.md");
    let oversized = path("项目/甲/02-日志/巨大附录.md");
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            note(journal.as_str(), "见 [[巨大附录]]"),
            degraded(oversized.as_str(), DocumentUnavailable::TooLarge),
            note("项目/乙/巨大附录.md", "# 乙的同名页"),
        ])
        .expect("rebuild succeeds");

    let document = index
        .document(&journal)
        .expect("query succeeds")
        .expect("indexed");
    assert_eq!(
        document.outgoing_links[0].resolution,
        LinkResolution::Resolved {
            path: oversized.clone(),
            rule: ResolutionRule::ExactRelative,
        },
        "降级只影响正文，不影响这条路径仍然存在这一事实"
    );
    assert_eq!(
        index
            .backlinks(&oversized)
            .expect("backlinks succeed")
            .len(),
        1,
        "指向降级笔记的入链必须照常建立"
    );
}

#[test]
fn diagnostics_belong_to_one_generation_and_disappear_when_the_note_becomes_readable() {
    let recovered = path("项目/甲/外链笔记.md");
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[degraded(recovered.as_str(), DocumentUnavailable::Symlink)])
        .expect("rebuild succeeds");
    assert_eq!(index.diagnostics().len(), 1);

    index
        .rebuild(&[note(recovered.as_str(), "# 恢复了\n见 [[外链笔记]]")])
        .expect("rebuild succeeds");

    assert!(
        index.diagnostics().is_empty(),
        "上一代的诊断绝不能沿用到本代"
    );
    let document = index
        .document(&recovered)
        .expect("query succeeds")
        .expect("indexed");
    assert_eq!(document.unavailable, None);
    assert_eq!(document.title.text, "恢复了");
}

#[test]
fn the_cached_rebuild_reports_the_same_diagnostics_without_caching_the_degradation() {
    let healthy = path("项目/甲/04-协调/TASKS.md");
    let degrading = path("项目/甲/外链笔记.md");
    let mut index = InMemoryVaultIndex::new();

    let (_, cache, _) = index
        .rebuild_cached(
            &[
                note(healthy.as_str(), "# 甲任务"),
                note(degrading.as_str(), "# 外链笔记"),
            ],
            None,
        )
        .expect("cold cached rebuild succeeds");
    assert!(index.diagnostics().is_empty());

    // 同一篇内容不变、但本轮读不进来：降级是本轮输入事实，不能被上一轮缓存盖过去。
    let (_, _, stats) = index
        .rebuild_cached(
            &[
                note(healthy.as_str(), "# 甲任务"),
                degraded(degrading.as_str(), DocumentUnavailable::Symlink),
            ],
            Some(&cache),
        )
        .expect("warm cached rebuild succeeds");

    assert_eq!(stats.reused, 1, "没降级的那篇仍可复用缓存解析");
    assert_eq!(
        index.diagnostics(),
        vec![IndexDiagnostic {
            path: degrading.clone(),
            reason: DocumentUnavailable::Symlink,
        }]
    );
    let document = index
        .document(&degrading)
        .expect("query succeeds")
        .expect("indexed");
    assert_eq!(document.unavailable, Some(DocumentUnavailable::Symlink));
}
