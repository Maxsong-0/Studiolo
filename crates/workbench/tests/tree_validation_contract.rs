//! 文件树构建错误契约；公开构建 Interface 对不完整批次整体失败关闭。

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::{FileTree, TreeError, TreeInput, TreeKind};

fn entry(path: &str, kind: TreeKind, size_bytes: Option<u64>) -> TreeInput {
    TreeInput {
        path: VaultPath::parse(path).unwrap(),
        kind,
        size_bytes,
    }
}

fn note(path: &str, content: &str) -> MarkdownDocument {
    MarkdownDocument::new(VaultPath::parse(path).unwrap(), content)
}

fn invalid(entries: &[TreeInput], documents: &[MarkdownDocument]) {
    assert!(matches!(
        FileTree::build(entries, documents),
        Err(TreeError::InvalidInput)
    ));
}

#[test]
fn duplicate_and_casefold_colliding_paths_are_rejected() {
    let file = entry("a.bin", TreeKind::Attachment, Some(1));
    invalid(&[file.clone(), file], &[]);
    invalid(
        &[
            entry("目录", TreeKind::Directory, None),
            entry("目录", TreeKind::Attachment, Some(0)),
        ],
        &[],
    );
    invalid(
        &[
            entry("A", TreeKind::Directory, None),
            entry("a", TreeKind::Directory, None),
        ],
        &[],
    );
    invalid(
        &[
            entry("Ä.bin", TreeKind::Attachment, Some(0)),
            entry("ä.bin", TreeKind::Attachment, Some(0)),
        ],
        &[],
    );
}

#[test]
fn all_hidden_components_are_rejected_not_only_protected_names() {
    for path in [
        ".git",
        ".workbench",
        ".obsidian",
        ".stignore",
        ".任意目录",
        "可见/.隐藏",
    ] {
        invalid(&[entry(path, TreeKind::Directory, None)], &[]);
    }
    invalid(
        &[entry(".秘密.md", TreeKind::Markdown, Some(0))],
        &[note(".秘密.md", "")],
    );
}

#[test]
fn every_parent_must_exist_and_be_a_directory() {
    invalid(
        &[entry("缺失/附件.bin", TreeKind::Attachment, Some(1))],
        &[],
    );
    invalid(
        &[
            entry("根/中间/深层", TreeKind::Directory, None),
            entry("根", TreeKind::Directory, None),
        ],
        &[],
    );
    invalid(
        &[
            entry("文件", TreeKind::Attachment, Some(1)),
            entry("文件/附件.bin", TreeKind::Attachment, Some(1)),
        ],
        &[],
    );
}

#[test]
fn directory_and_file_sizes_are_not_interchangeable() {
    invalid(&[entry("目录", TreeKind::Directory, Some(0))], &[]);
    invalid(&[entry("附件.bin", TreeKind::Attachment, None)], &[]);
    invalid(
        &[entry("文档.md", TreeKind::Markdown, None)],
        &[note("文档.md", "")],
    );
}

#[test]
fn extensions_and_kinds_cannot_misrepresent_markdown() {
    invalid(
        &[entry("伪装.txt", TreeKind::Markdown, Some(0))],
        &[note("伪装.txt", "")],
    );
    invalid(&[entry("文档.MD", TreeKind::Attachment, Some(0))], &[]);
    invalid(
        &[entry("文档.md.tmp", TreeKind::Markdown, Some(0))],
        &[note("文档.md.tmp", "")],
    );
    let entries = [
        entry("目录.md", TreeKind::Directory, None),
        entry("目录.md/文档.Md", TreeKind::Markdown, Some(0)),
    ];
    assert!(FileTree::build(&entries, &[note("目录.md/文档.Md", "")]).is_ok());
}

#[test]
fn markdown_sets_match_exactly_and_duplicates_are_rejected() {
    let entries = [entry("a.md", TreeKind::Markdown, Some(0))];
    invalid(&entries, &[]);
    invalid(&[], &[note("a.md", "")]);
    invalid(&entries, &[note("b.md", "")]);
    invalid(&entries, &[note("a.md", ""), note("a.md", "")]);
    invalid(
        &[entry("a.md", TreeKind::Directory, None)],
        &[note("a.md", "")],
    );
    invalid(
        &[entry("a.txt", TreeKind::Attachment, Some(0))],
        &[note("a.txt", "")],
    );
}

#[test]
fn markdown_sizes_use_exact_utf8_bytes_including_bom_and_crlf() {
    let content = "\u{feff}# 中文🗂️\r\n";
    let documents = [note("中文.MD", content)];
    let bytes = content.len() as u64;
    let tree = FileTree::build(
        &[entry("中文.MD", TreeKind::Markdown, Some(bytes))],
        &documents,
    )
    .unwrap();
    assert_eq!(
        tree.page("", 0).unwrap().entries[0].size_bytes,
        Some(bytes.to_string())
    );
    invalid(
        &[entry(
            "中文.MD",
            TreeKind::Markdown,
            Some(content.chars().count() as u64),
        )],
        &documents,
    );
    invalid(
        &[entry("中文.MD", TreeKind::Markdown, Some(bytes - 1))],
        &documents,
    );
    invalid(
        &[entry("中文.MD", TreeKind::Markdown, Some(bytes + 1))],
        &documents,
    );
}
