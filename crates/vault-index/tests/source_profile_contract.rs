//! 参考性来源分级的公开接缝契约测试。

use studiolo_core::{Classification, VaultPath};
use studiolo_vault_index::{InMemoryVaultIndex, MarkdownDocument, SourceProvenance, VaultIndex};

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("fixture path is valid")
}

#[test]
fn public_index_seam_keeps_declared_source_facts_but_defaults_untrusted_notes_to_restricted() {
    let human_path = path("项目/研究/私人笔记.md");
    let source_path = path("sources/web/T-1/A-1/证据.md");
    let draft_path = path("projects/研究/drafts/A-1.md");
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            MarkdownDocument::new(
                human_path.clone(),
                "---\nclassification: public\n---\n# 人写内容",
            ),
            MarkdownDocument::new(
                source_path.clone(),
                "---\nsource: NASA\nsource_type: web\nclassification: public\n---\n# 外部资料",
            ),
            MarkdownDocument::new(draft_path.clone(), "# Agent 草稿"),
        ])
        .expect("index rebuild succeeds");

    let human = index
        .source_profile(&human_path)
        .expect("source profile query succeeds");
    assert_eq!(human.provenance, SourceProvenance::HumanUnclassified);
    assert_eq!(human.declared_classification, Some(Classification::Public));
    assert_eq!(human.safe_default, Classification::Restricted);

    let source = index
        .source_profile(&source_path)
        .expect("source profile query succeeds");
    assert_eq!(source.provenance, SourceProvenance::ExternalDeclared);
    assert_eq!(source.source.as_deref(), Some("NASA"));
    assert_eq!(source.source_type.as_deref(), Some("web"));
    assert_eq!(source.declared_classification, Some(Classification::Public));
    assert_eq!(source.safe_default, Classification::Restricted);

    let draft = index
        .source_profile(&draft_path)
        .expect("source profile query succeeds");
    assert_eq!(draft.provenance, SourceProvenance::AgentArtifact);
    assert_eq!(draft.safe_default, Classification::Restricted);
}
