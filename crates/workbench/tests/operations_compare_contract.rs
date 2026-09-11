//! 目录漂移只认可完整观测；双向范围与未知状态的回归契约。

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::{DirectoryObservation, ProjectOperations, WorkbenchReadModel};

fn declaration() -> ProjectOperations {
    WorkbenchReadModel::build(&[MarkdownDocument::new(
        VaultPath::parse("项目/甲/01-框架/目录结构.md").unwrap(),
        "## 代码仓库\n```text\nrepo/\n├── crates/\n│   └── core/ # 核心模块\n├── docs/\n└── web/\n```",
    )]).unwrap().project_operations("项目/甲").unwrap()
}

fn observation(path: &str, status: &str, children: &[&str]) -> DirectoryObservation {
    DirectoryObservation {
        path: path.to_owned(),
        status: status.to_owned(),
        children: children.iter().map(|path| (*path).to_owned()).collect(),
    }
}

fn statuses(
    value: &ProjectOperations,
    observations: &[DirectoryObservation],
) -> Vec<(String, String)> {
    value
        .compare(observations)
        .into_iter()
        .map(|drift| (drift.path, drift.status))
        .collect()
}

#[test]
fn full_observations_detect_both_directions_only_inside_expanded_parents() {
    let value = declaration();
    let observed = [
        observation("", "ready", &["crates", "docs", "scripts"]),
        observation("crates", "ready", &["crates/extra"]),
    ];
    assert_eq!(
        statuses(&value, &observed),
        [
            ("crates".into(), "present".into()),
            ("crates/core".into(), "missing".into()),
            ("crates/extra".into(), "undocumented".into()),
            ("docs".into(), "present".into()),
            ("scripts".into(), "undocumented".into()),
            ("web".into(), "missing".into()),
        ]
    );
    assert_eq!(value.comparison_parents, ["", "crates"]);
    let result = value.compare(&observed);
    assert_eq!(
        result
            .iter()
            .find(|drift| drift.path == "crates/core")
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .line,
        5
    );
    assert!(
        result
            .iter()
            .find(|drift| drift.path == "scripts")
            .unwrap()
            .source
            .is_none()
    );
    let mut reordered = observed.to_vec();
    reordered.reverse();
    reordered[1].children.reverse();
    assert_eq!(value.compare(&reordered), result);
}

#[test]
fn no_binding_or_missing_parent_observation_does_not_manufacture_absence() {
    let value = declaration();
    assert!(
        value
            .compare(&[])
            .iter()
            .all(|drift| drift.status == "unverified")
    );
    let root = observation("", "ready", &["crates", "docs", "web"]);
    let absent = statuses(&value, std::slice::from_ref(&root));
    assert_eq!(
        absent
            .iter()
            .find(|(path, _)| path == "crates/core")
            .unwrap()
            .1,
        "unverified"
    );
    let unavailable = observation("crates", "unavailable", &[]);
    assert_eq!(statuses(&value, &[root, unavailable]), absent);
}

#[test]
fn confirmed_missing_ancestors_prove_missing_descendants() {
    let value = declaration();
    let missing_root = observation("", "missing", &[]);
    assert!(
        value
            .compare(&[missing_root])
            .iter()
            .all(|drift| drift.status == "missing")
    );
    let root = observation("", "ready", &["docs", "web"]);
    assert_eq!(
        statuses(&value, &[root])
            .iter()
            .find(|(path, _)| path == "crates/core")
            .unwrap()
            .1,
        "missing"
    );
    let absent_parent = observation("crates", "missing", &[]);
    let result = statuses(&value, &[absent_parent]);
    assert_eq!(
        result
            .iter()
            .find(|(path, _)| path == "crates/core")
            .unwrap()
            .1,
        "missing"
    );
    assert_eq!(
        result.iter().find(|(path, _)| path == "docs").unwrap().1,
        "unverified"
    );
}

#[test]
fn inconsistent_parent_and_child_evidence_is_unverified() {
    let value = declaration();
    let result = statuses(
        &value,
        &[
            observation("", "ready", &["docs", "web"]),
            observation("crates", "ready", &["crates/core"]),
        ],
    );
    for path in ["crates", "crates/core"] {
        assert_eq!(
            result
                .iter()
                .find(|(candidate, _)| candidate == path)
                .unwrap()
                .1,
            "unverified"
        );
    }
}

#[test]
fn malformed_partial_duplicate_or_out_of_scope_observations_cannot_produce_green_results() {
    let value = declaration();
    let root = observation("", "ready", &["crates", "docs", "web"]);
    for invalid in [
        root.clone(),
        observation("crates", "ready", &["../escape"]),
        observation("crates", "ready", &["crates/.hidden"]),
        observation("crates", "ready", &["crates/core/deeper"]),
        observation("crates", "ready", &["crates/core", "crates/core"]),
        observation("crates", "ready", &["crates/Core", "crates/core"]),
        observation("crates", "missing", &["crates/core"]),
        observation("crates", "unavailable", &["crates/core"]),
        observation("crates", "partial", &["crates/core"]),
        observation("docs", "ready", &["docs/outside-scope"]),
    ] {
        let result = value.compare(&[root.clone(), invalid]);
        assert_eq!(result.len(), value.directories.len());
        assert!(result.iter().all(|drift| drift.status == "unverified"));
    }
}

#[test]
fn invalid_declaration_does_not_use_observations_even_if_it_retains_nodes() {
    let mut value = declaration();
    value.declaration_status = "invalid".to_owned();
    let result = value.compare(&[observation(
        "",
        "ready",
        &["crates", "docs", "web", "extra"],
    )]);
    assert_eq!(result.len(), value.directories.len());
    assert!(result.iter().all(|drift| drift.status == "unverified"));
}
