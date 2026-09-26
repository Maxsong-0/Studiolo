//! 公开接口回归：导航安全的 Markdown 解析与路径匹配。

use studiolo_core::VaultPath;
use studiolo_vault_index::{InMemoryVaultIndex, LinkResolution, MarkdownDocument, VaultIndex};

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

fn note(value: &str, body: &str) -> MarkdownDocument {
    MarkdownDocument::new(path(value), body)
}

#[test]
fn headings_and_links_share_strict_fences_and_preserve_bom_crlf_sources() {
    let lines = [
        "\u{feff}---",
        "title: 可见标题",
        "---",
        "# 可见",
        "``` rust",
        "[[伪1]]",
        "~~~",
        "## 伪标题",
        "``",
        "[[伪2]]",
        "``` trailing",
        "[[伪3]]",
        "```",
        "````md",
        "```",
        "[[伪4]]",
        "````",
        r"😀 [[真实]] `[[伪5]]` ``[[伪6]]`` ```[[伪7]]``` \[[伪8]] \![[普通]] ![[嵌入]] \\[[偶数]]",
        "## 可见二",
        "~~~",
        "[[伪9]]",
    ];
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            note("来源.md", &lines.join("\r\n")),
            note("真实.md", "# 真实"),
        ])
        .unwrap();
    let document = index.document(&path("来源.md")).unwrap().unwrap();
    assert_eq!(document.title.text, "可见标题");
    assert_eq!(
        document
            .headings
            .iter()
            .map(|heading| heading.line)
            .collect::<Vec<_>>(),
        vec![4, 19]
    );
    assert_eq!(
        document
            .outgoing_links
            .iter()
            .map(|link| link.target.target.as_str())
            .collect::<Vec<_>>(),
        vec!["真实", "普通", "嵌入", "偶数"]
    );
    assert_eq!(document.outgoing_links[0].line, 18);
    assert_eq!(document.outgoing_links[0].column, 3);
    assert_eq!(document.outgoing_links[0].context, lines[17]);
    assert!(!document.outgoing_links[1].target.embed);
    assert!(document.outgoing_links[2].target.embed);
    let backlinks = index.backlinks(&path("真实.md")).unwrap();
    assert_eq!(backlinks.len(), 1);
    assert_eq!((backlinks[0].line, backlinks[0].column), (18, 3));
}

#[test]
fn equal_length_inline_delimiters_and_escape_parity_do_not_hide_real_links() {
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[note(
            "源.md",
            r"``code ` [[伪1]] ` code`` [[真1]] \` [[真2]] ` unmatched [[真3]]",
        )])
        .unwrap();
    let document = index.document(&path("源.md")).unwrap().unwrap();
    assert_eq!(
        document
            .outgoing_links
            .iter()
            .map(|link| link.target.target.as_str())
            .collect::<Vec<_>>(),
        vec!["真1", "真2", "真3"]
    );
}

#[test]
fn case_folded_exact_path_collisions_are_ambiguous_and_never_create_backlinks() {
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            note(
                "入口.md",
                "[[Doc]] [[doc.md]] [[/Doc]] [[Only.MD]] [[missing/Doc]]",
            ),
            note("Doc.md", "# 大写"),
            note("doc.md", "# 小写"),
            note("Only.MD", "# 单页"),
        ])
        .unwrap();
    let document = index.document(&path("入口.md")).unwrap().unwrap();
    for link in &document.outgoing_links[..3] {
        assert_eq!(
            link.resolution,
            LinkResolution::Ambiguous {
                candidates: vec![path("Doc.md"), path("doc.md")]
            }
        );
    }
    assert!(
        matches!(&document.outgoing_links[3].resolution, LinkResolution::Resolved { path: target, .. } if *target == path("Only.MD"))
    );
    assert_eq!(
        document.outgoing_links[4].resolution,
        LinkResolution::Dangling
    );
    assert!(index.backlinks(&path("Doc.md")).unwrap().is_empty());
    assert!(index.backlinks(&path("doc.md")).unwrap().is_empty());
    assert_eq!(index.link_health().unwrap().ambiguous.len(), 3);
}

#[test]
fn nested_list_items_and_continuations_keep_their_links() {
    // Obsidian 默认用 tab 缩进嵌套列表；用空格时通常是四个。两种写法都是列表内容，不是缩进代码块。
    let lines = [
        "- 上级条目",
        "\t- 子项见 [[Tab缩进]]",
        "    - 子项见 [[四空格]]",
        "      续行也有 [[续行链接]]",
        "",
        "```",
        "    [[围栏内仍是伪链接]]",
        "```",
    ];
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            note("列表.md", &lines.join("\n")),
            note("Tab缩进.md", "# t"),
            note("四空格.md", "# s"),
            note("续行链接.md", "# c"),
        ])
        .unwrap();
    let document = index.document(&path("列表.md")).unwrap().unwrap();
    assert_eq!(
        document
            .outgoing_links
            .iter()
            .map(|link| (link.target.target.as_str(), link.line))
            .collect::<Vec<_>>(),
        vec![("Tab缩进", 2), ("四空格", 3), ("续行链接", 4)]
    );
    for target in ["Tab缩进.md", "四空格.md", "续行链接.md"] {
        assert_eq!(
            index.backlinks(&path(target)).unwrap().len(),
            1,
            "{target} lost its backlink from a nested list item"
        );
    }
}
