//! 文件树公开 Interface 契约；所有夹具只在内存中，不依赖真实源目录。

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::{FileTree, TreeError, TreeInput, TreeKind, TreePage};

fn entry(path: &str, kind: TreeKind, size_bytes: Option<u64>) -> TreeInput {
    TreeInput {
        path: VaultPath::parse(path).unwrap(),
        kind,
        size_bytes,
    }
}

#[test]
fn empty_root_has_one_empty_terminal_page() {
    let tree = FileTree::build(&[], &[]).unwrap();
    assert_eq!(
        tree.page("", 0).unwrap(),
        TreePage {
            path: String::new(),
            offset: 0,
            total: 0,
            entries: Vec::new(),
            next_offset: None,
        }
    );
    assert_eq!(tree.page("", 1), Err(TreeError::InvalidOffset));
}

#[test]
fn empty_and_attachment_only_directories_keep_direct_child_counts() {
    let entries = [
        entry("空目录", TreeKind::Directory, None),
        entry("附件", TreeKind::Directory, None),
        entry("附件/二级", TreeKind::Directory, None),
        entry("附件/二级/图片.png", TreeKind::Attachment, Some(4)),
        entry("附件/说明.pdf", TreeKind::Attachment, Some(0)),
    ];
    let tree = FileTree::build(&entries, &[]).unwrap();
    let root = tree.page("", 0).unwrap();
    assert_eq!(root.total, 2);
    assert_eq!(root.entries[0].child_count, Some(0));
    assert_eq!(root.entries[1].child_count, Some(2));
    assert_eq!(root.entries[1].size_bytes, None);
    assert!(tree.page("空目录", 0).unwrap().entries.is_empty());
    assert_eq!(tree.page("空目录", 1), Err(TreeError::InvalidOffset));
    let attachments = tree.page("附件", 0).unwrap();
    assert_eq!(attachments.entries[0].child_count, Some(1));
    assert_eq!(attachments.entries[1].child_count, None);
    assert_eq!(attachments.entries[1].size_bytes.as_deref(), Some("0"));
}

#[test]
fn directories_precede_files_then_full_paths_define_stable_order() {
    let mut entries = vec![
        entry("b.md", TreeKind::Markdown, Some(0)),
        entry("z目录", TreeKind::Directory, None),
        entry("a.png", TreeKind::Attachment, Some(7)),
        entry("a目录", TreeKind::Directory, None),
        entry("Z.pdf", TreeKind::Attachment, Some(8)),
    ];
    let documents = [MarkdownDocument::new(VaultPath::parse("b.md").unwrap(), "")];
    let first = FileTree::build(&entries, &documents)
        .unwrap()
        .page("", 0)
        .unwrap();
    entries.reverse();
    let second = FileTree::build(&entries, &documents)
        .unwrap()
        .page("", 0)
        .unwrap();
    assert_eq!(first, second);
    let paths: Vec<_> = first
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(paths, ["a目录", "z目录", "Z.pdf", "a.png", "b.md"]);
}

#[test]
fn ten_thousand_nodes_page_without_duplicates_or_omissions() {
    let entries: Vec<_> = (0..10_000)
        .rev()
        .map(|number| {
            entry(
                &format!("附件-{number:05}.png"),
                TreeKind::Attachment,
                Some(1),
            )
        })
        .collect();
    let tree = FileTree::build(&entries, &[]).unwrap();
    let mut next = Some(0);
    let mut paths = Vec::new();
    let mut pages = 0;
    while let Some(offset) = next {
        let page = tree.page("", offset).unwrap();
        assert_eq!(page.offset, offset);
        assert_eq!(page.total, 10_000);
        assert_eq!(page.entries.len(), 200);
        next = page.next_offset;
        paths.extend(page.entries.into_iter().map(|entry| entry.path));
        pages += 1;
    }
    assert_eq!(pages, 50);
    let expected: Vec<_> = (0..10_000)
        .map(|number| format!("附件-{number:05}.png"))
        .collect();
    assert_eq!(paths, expected);
    assert_eq!(tree.page("", 10_000), Err(TreeError::InvalidOffset));
    assert_eq!(tree.page("", usize::MAX), Err(TreeError::InvalidOffset));
}

#[test]
fn partial_pages_and_unaligned_offsets_are_bounded() {
    let entries: Vec<_> = (0..201)
        .map(|number| entry(&format!("{number:03}.bin"), TreeKind::Attachment, Some(0)))
        .collect();
    let tree = FileTree::build(&entries, &[]).unwrap();
    assert_eq!(tree.page("", 0).unwrap().next_offset, Some(200));
    let last = tree.page("", 200).unwrap();
    assert_eq!(last.entries.len(), 1);
    assert_eq!(last.entries[0].name, "200.bin");
    assert_eq!(last.next_offset, None);
    let shifted = tree.page("", 1).unwrap();
    assert_eq!(shifted.entries.len(), 200);
    assert_eq!(shifted.next_offset, None);
    assert_eq!(tree.page("", 201), Err(TreeError::InvalidOffset));
}

#[test]
fn parents_are_order_independent_and_prefixes_do_not_include_descendants() {
    let entries = [
        entry("a/b/c.bin", TreeKind::Attachment, Some(1)),
        entry("ab/兄弟.bin", TreeKind::Attachment, Some(1)),
        entry("a/b", TreeKind::Directory, None),
        entry("ab", TreeKind::Directory, None),
        entry("a", TreeKind::Directory, None),
    ];
    let tree = FileTree::build(&entries, &[]).unwrap();
    let page = tree.page("a", 0).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.entries[0].path, "a/b");
    assert_eq!(page.entries[0].child_count, Some(1));
    assert_eq!(tree.page("a/b", 0).unwrap().entries[0].path, "a/b/c.bin");
    assert_eq!(tree.page("ab", 0).unwrap().entries[0].path, "ab/兄弟.bin");
}

#[test]
fn visible_canonical_query_errors_are_distinct() {
    let tree = FileTree::build(&[entry("文件.pdf", TreeKind::Attachment, Some(1))], &[]).unwrap();
    for invalid in [
        "/",
        ".",
        "..",
        "目录/",
        "a//b",
        "a/../b",
        "a\\b",
        ".隐藏",
        "a/.隐藏",
        "a%2fb",
        "a%2Eb",
        "e\u{301}",
        "a\n",
    ] {
        assert_eq!(
            tree.page(invalid, 0),
            Err(TreeError::InvalidPath),
            "{invalid:?}"
        );
    }
    assert_eq!(tree.page("不存在", 0), Err(TreeError::NotFound));
    assert_eq!(tree.page("不存在", usize::MAX), Err(TreeError::NotFound));
    assert_eq!(tree.page("文件.pdf", 0), Err(TreeError::NotDirectory));
    assert_eq!(
        tree.page("文件.pdf", usize::MAX),
        Err(TreeError::NotDirectory)
    );
}

#[test]
fn owned_pages_and_large_byte_strings_need_no_external_files() {
    let tree = {
        let entries = [entry(
            "纯内存附件.bin",
            TreeKind::Attachment,
            Some(u64::MAX),
        )];
        FileTree::build(&entries, &[]).unwrap()
    };
    let mut page = tree.page("", 0).unwrap();
    assert_eq!(
        page.entries[0].size_bytes.as_deref(),
        Some("18446744073709551615")
    );
    page.entries[0].path.clear();
    page.entries.clear();
    assert_eq!(tree.page("", 0).unwrap().entries[0].path, "纯内存附件.bin");
    fn serializable<T: serde::Serialize>() {}
    serializable::<TreePage>();
    serializable::<TreeInput>();
    serializable::<TreeKind>();
    let spaced = FileTree::build(
        &[entry("预组合é 与 空格.bin", TreeKind::Attachment, Some(1))],
        &[],
    )
    .unwrap();
    assert_eq!(
        spaced.page("", 0).unwrap().entries[0].name,
        "预组合é 与 空格.bin"
    );
}
