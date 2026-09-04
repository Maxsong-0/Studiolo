//! Public-seam contract tests for frontmatter and heading outlines.

use studiolo_core::VaultPath;
use studiolo_vault_index::{
    FRONTMATTER_KEY_ALIASES, FRONTMATTER_SPECIAL_KEY_ALIASES, FrontmatterValue, InMemoryVaultIndex,
    MarkdownDocument, NormalizedStatus, TitleSource, VaultIndex,
};

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("fixture path is valid")
}

#[test]
fn public_index_seam_normalizes_frontmatter_and_builds_a_unicode_heading_tree() {
    let note_path = path("项目/火箭/设计.md");
    let mut index = InMemoryVaultIndex::new();
    let input = MarkdownDocument::new(
        note_path.clone(),
        r#"---
标题: 火箭 🚀
title: Canonical title wins
aliases: [星舰, "Star Ship"]
tags:
  - 航天
  - Rust
status: done-pending-restart
状态: "✅ **代码完成+全栈验证·待用户重启**"
date: "2026-09-03"
日期: "2025-01-01"
project: Studiolo
项目: 旧项目名
type: followup-suggestions
类型: 架构文档
id: ADR-001
编号: ADR-旧值
severity: P0
严重度: P1
严重级: P2
updated: "2026-09-03"
更新: "2026-09-01"
更新日期: "2026-09-02"
created: "2026-08-01"
创建: "2026-07-01"
发现日期: "2026-08-10"
修复日期: "2026-08-11"
source: NASA 中文资料
source_type: web
custom_key: 保留我
---
# 总览
## 设计 α
### 推进器
## 安全
"#,
    );

    let generation = index.rebuild(&[input]).expect("index rebuild succeeds");
    assert_eq!(generation.documents, 1);

    let document = index
        .document(&note_path)
        .expect("query succeeds")
        .expect("document is indexed");
    assert_eq!(document.title.text, "Canonical title wins");
    assert_eq!(document.title.source, TitleSource::Frontmatter);
    assert_eq!(
        FRONTMATTER_KEY_ALIASES
            .iter()
            .map(|group| (group.canonical, group.aliases))
            .collect::<Vec<_>>(),
        vec![
            ("status", &["状态"][..]),
            ("date", &["日期"][..]),
            ("project", &["项目"][..]),
            ("type", &["类型"][..]),
            ("id", &["编号"][..]),
            ("severity", &["严重度", "严重级"][..]),
            ("updated", &["更新", "更新日期"][..]),
            ("created", &["创建"][..]),
        ]
    );
    assert_eq!(
        FRONTMATTER_SPECIAL_KEY_ALIASES
            .iter()
            .map(|group| (group.canonical, group.aliases))
            .collect::<Vec<_>>(),
        vec![
            ("discovered_at", &["发现日期"][..]),
            ("fixed_at", &["修复日期"][..]),
        ]
    );
    assert_eq!(
        document.frontmatter.get("status"),
        Some(&FrontmatterValue::Status(NormalizedStatus::Pending))
    );
    assert_eq!(
        document.frontmatter.get("aliases"),
        Some(&FrontmatterValue::List(vec![
            FrontmatterValue::Text("星舰".into()),
            FrontmatterValue::Text("Star Ship".into()),
        ]))
    );
    assert_eq!(
        document.frontmatter.get("tags"),
        Some(&FrontmatterValue::List(vec![
            FrontmatterValue::Text("航天".into()),
            FrontmatterValue::Text("Rust".into()),
        ]))
    );
    assert_eq!(
        document.frontmatter.get("type"),
        Some(&FrontmatterValue::Text("follow-up".into()))
    );
    assert_eq!(
        document.frontmatter.get("discovered_at"),
        Some(&FrontmatterValue::Text("2026-08-10".into()))
    );
    assert_eq!(
        document.frontmatter.get("fixed_at"),
        Some(&FrontmatterValue::Text("2026-08-11".into()))
    );

    let raw_status = document
        .frontmatter
        .entries
        .iter()
        .find(|entry| entry.raw_key == "状态")
        .expect("raw Chinese key is retained");
    assert_eq!(raw_status.canonical_key, "status");
    assert_eq!(
        raw_status.raw_value,
        "\"✅ **代码完成+全栈验证·待用户重启**\""
    );
    let raw_chinese_title = document
        .frontmatter
        .entries
        .iter()
        .find(|entry| entry.raw_key == "标题")
        .expect("unknown Chinese title key is retained");
    assert_eq!(raw_chinese_title.canonical_key, "标题");
    assert!(document.frontmatter.entries.iter().any(|entry| {
        entry.raw_key == "custom_key"
            && entry.canonical_key == "custom_key"
            && entry.raw_value == "保留我"
    }));
    assert!(
        document
            .frontmatter
            .warnings
            .iter()
            .any(|warning| warning.canonical_key() == Some("severity"))
    );
    assert!(
        !document
            .frontmatter
            .warnings
            .iter()
            .any(|warning| warning.canonical_key() == Some("title"))
    );

    let tree = index
        .heading_tree(&note_path)
        .expect("heading query succeeds");
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].heading.text, "总览");
    assert_eq!(tree[0].children.len(), 2);
    assert_eq!(tree[0].children[0].heading.text, "设计 α");
    assert_eq!(tree[0].children[0].children[0].heading.text, "推进器");
    assert_eq!(tree[0].children[1].heading.text, "安全");
}

#[test]
fn status_normalization_uses_the_authoritative_closed_match_table() {
    let fixtures = [
        ("已采纳（生效）", NormalizedStatus::Adopted),
        ("✅ **已修复**", NormalizedStatus::Fixed),
        ("完成 # archived", NormalizedStatus::Done),
        ("🚧 进行中", NormalizedStatus::InProgress),
        ("⏳ **待执行（下一步）**", NormalizedStatus::Pending),
        ("设计已批+冻结", NormalizedStatus::Approved),
        ("已被取代·ADR-2", NormalizedStatus::Superseded),
        ("已废弃", NormalizedStatus::Deprecated),
        ("构想", NormalizedStatus::Draft),
        ("approved", NormalizedStatus::Unknown("approved".to_owned())),
        ("pending", NormalizedStatus::Unknown("pending".to_owned())),
    ];
    let inputs: Vec<_> = fixtures
        .iter()
        .enumerate()
        .map(|(index, (status, _))| {
            MarkdownDocument::new(
                path(&format!("状态/{index}.md")),
                format!("---\nstatus: {status:?}\n---\n"),
            )
        })
        .collect();
    let mut index = InMemoryVaultIndex::new();
    index.rebuild(&inputs).expect("index rebuild succeeds");

    for (fixture_index, (_, expected)) in fixtures.iter().enumerate() {
        let document = index
            .document(&path(&format!("状态/{fixture_index}.md")))
            .expect("query succeeds")
            .expect("fixture is indexed");
        assert_eq!(
            document.frontmatter.get("status"),
            Some(&FrontmatterValue::Status(expected.clone()))
        );
    }
}
