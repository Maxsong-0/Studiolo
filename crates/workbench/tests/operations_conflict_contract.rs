//! 祖先的存在性证据矛盾须传播，缺少证据本身不制造矛盾或叶节点存在性。

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::{
    DirectoryDrift, DirectoryObservation, ProjectOperations, WorkbenchReadModel,
};

fn declaration() -> ProjectOperations {
    WorkbenchReadModel::build(&[MarkdownDocument::new(
        VaultPath::parse("项目/甲/01-框架/目录结构.md").unwrap(),
        "## 代码仓库\n```text\nrepo/\n├── src/\n│   └── mid/\n│       └── leaf/\n│           └── child/\n└── docs/\n```",
    )]).unwrap().project_operations("项目/甲").unwrap()
}

fn observation(path: &str, status: &str, children: &[&str]) -> DirectoryObservation {
    DirectoryObservation {
        path: path.to_owned(),
        status: status.to_owned(),
        children: children.iter().map(|child| (*child).to_owned()).collect(),
    }
}

fn status<'a>(result: &'a [DirectoryDrift], path: &str) -> &'a str {
    result
        .iter()
        .find(|item| item.path == path)
        .unwrap()
        .status
        .as_str()
}

#[test]
fn parent_listed_present_and_reported_missing_invalidates_its_whole_branch() {
    let result = declaration().compare(&[
        observation("", "ready", &["src", "docs", "outside"]),
        observation("src", "missing", &[]),
        observation("src/mid", "ready", &["src/mid/extra"]),
    ]);
    for path in [
        "src",
        "src/mid",
        "src/mid/leaf",
        "src/mid/leaf/child",
        "src/mid/extra",
    ] {
        assert_eq!(status(&result, path), "unverified", "{path}");
    }
    assert_eq!(status(&result, "docs"), "present");
    assert_eq!(status(&result, "outside"), "undocumented");
}

#[test]
fn missing_ancestor_and_deep_ready_observation_are_inconsistent_in_both_directions() {
    let result = declaration().compare(&[
        observation("", "ready", &["docs"]),
        observation(
            "src/mid/leaf",
            "ready",
            &["src/mid/leaf/child", "src/mid/leaf/extra"],
        ),
    ]);
    for path in [
        "src",
        "src/mid",
        "src/mid/leaf",
        "src/mid/leaf/child",
        "src/mid/leaf/extra",
    ] {
        assert_eq!(status(&result, path), "unverified", "{path}");
    }
    assert_eq!(status(&result, "docs"), "present");
}

#[test]
fn unknown_ancestors_do_not_negate_an_explicit_missing_leaf() {
    let result = declaration().compare(&[
        observation("", "unavailable", &[]),
        observation("src", "unavailable", &[]),
        observation("src/mid/leaf", "missing", &[]),
    ]);
    for path in ["src", "src/mid", "docs"] {
        assert_eq!(status(&result, path), "unverified");
    }
    assert_eq!(status(&result, "src/mid/leaf"), "missing");
    assert_eq!(status(&result, "src/mid/leaf/child"), "missing");
}

#[test]
fn ancestors_existing_does_not_prove_unobserved_leaf_nodes_exist() {
    let result = declaration().compare(&[
        observation("", "ready", &["src", "docs"]),
        observation("src", "ready", &["src/mid"]),
        observation("src/mid", "unavailable", &[]),
    ]);
    for path in ["src", "src/mid", "docs"] {
        assert_eq!(status(&result, path), "present");
    }
    assert_eq!(status(&result, "src/mid/leaf"), "unverified");
    assert_eq!(status(&result, "src/mid/leaf/child"), "unverified");
}
