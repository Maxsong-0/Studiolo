//! 验证前缀选择只枚举指定子树，并正确处理相邻名称和 Unicode 路径。

use super::scoped_documents;
use crate::WorkbenchReadModel;
use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;

#[test]
fn scoped_iterator_visits_only_project_documents_among_many_neighbors() {
    let mut notes = (0..2000)
        .map(|i| {
            MarkdownDocument::new(
                VaultPath::parse(format!("项目/甲-{i:04}/笔记.md")).unwrap(),
                "说明",
            )
        })
        .collect::<Vec<_>>();
    for name in [
        "项目/甲/ 首篇.md",
        "项目/甲/深层/🌿.md",
        "项目/甲乙/末篇.md",
    ] {
        notes.push(MarkdownDocument::new(
            VaultPath::parse(name).unwrap(),
            "说明",
        ));
    }
    let model = WorkbenchReadModel::build(&notes).unwrap();
    let selected: Vec<_> = scoped_documents(&model.documents, Some("项目/甲/"))
        .map(|(path, _)| path.as_str())
        .collect();
    assert_eq!(selected, ["项目/甲/ 首篇.md", "项目/甲/深层/🌿.md"]);
    assert_eq!(
        scoped_documents(&model.documents, None).count(),
        notes.len()
    );
    assert_eq!(
        scoped_documents(&model.documents, Some("项目/不存在/")).count(),
        0
    );
}
