//! 白名单只读根契约：未列入的目录从一开始不进入计算；越界一律拒绝；失败关闭。

use std::fs;

use studiolo_core::VaultPath;
use studiolo_supply::{EntryKind, ScanLimits, SupplyError, SupplyRoots};
use tempfile::TempDir;

fn vault() -> TempDir {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("项目/Alpha/00-总览")).unwrap();
    fs::create_dir_all(root.join("项目/Beta")).unwrap();
    fs::create_dir_all(root.join("School")).unwrap();
    fs::create_dir_all(root.join("_规则")).unwrap();
    fs::write(root.join("项目/Alpha/00-总览/INDEX.md"), "# Alpha\n").unwrap();
    fs::write(root.join("项目/Alpha/附件.pdf"), b"%PDF").unwrap();
    fs::write(root.join("项目/Beta/INDEX.md"), "# Beta\n").unwrap();
    fs::write(root.join("School/secret.md"), "# 私人\n").unwrap();
    fs::write(root.join("_规则/通用工作规则.md"), "# 规则\n").unwrap();
    fs::write(root.join("账本.md"), "# 顶层私人\n").unwrap();
    temp
}

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

#[test]
fn only_allow_listed_prefixes_enter_the_snapshot() {
    let temp = vault();
    let roots = SupplyRoots::open(temp.path(), &["项目/Alpha", "_规则"]).unwrap();
    let snapshot = roots.scan(ScanLimits::default()).unwrap();
    let notes = snapshot
        .notes
        .iter()
        .map(|note| note.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        notes,
        ["_规则/通用工作规则.md", "项目/Alpha/00-总览/INDEX.md"]
    );
    assert!(snapshot.entries.iter().all(|entry| {
        entry.path.as_str().starts_with("项目/Alpha/") || entry.path.as_str().starts_with("_规则/")
    }));
    let attachment = snapshot
        .entries
        .iter()
        .find(|entry| entry.path.as_str() == "项目/Alpha/附件.pdf")
        .unwrap();
    assert_eq!(attachment.kind, EntryKind::Attachment);
    assert_eq!(attachment.size_bytes, Some(4));
    assert!(snapshot.diagnostics.is_empty());
    assert_eq!(snapshot.documents().len(), 2);
}

#[test]
fn reads_outside_the_allow_list_are_rejected_without_touching_the_filesystem() {
    let temp = vault();
    let roots = SupplyRoots::open(temp.path(), &["项目/Alpha"]).unwrap();
    for outside in [
        "School/secret.md",
        "账本.md",
        "项目/Beta/INDEX.md",
        "项目/AlphaX/x.md",
    ] {
        assert!(
            matches!(
                roots.read_note(&path(outside), 1 << 20),
                Err(SupplyError::OutsideAllowlist)
            ),
            "{outside} must be outside the allow list"
        );
        assert!(!roots.allows(&path(outside)));
    }
    assert!(matches!(
        roots.read_note(&path("项目/Alpha/missing.md"), 1 << 20),
        Err(SupplyError::NotFound)
    ));
    let note = roots
        .read_note(&path("项目/Alpha/00-总览/INDEX.md"), 1 << 20)
        .unwrap();
    assert_eq!(note.content, "# Alpha\n");
    assert_eq!(note.path.as_str(), "项目/Alpha/00-总览/INDEX.md");
}

#[test]
fn allow_list_configuration_fails_closed() {
    let temp = vault();
    for (allow, what) in [
        (vec![], "empty"),
        (vec![".obsidian"], "hidden"),
        (vec![".workbench"], "protected"),
        (vec!["项目/Alpha", "项目"], "nested"),
        (vec!["项目/Alpha", "项目/Alpha"], "duplicate"),
        (vec!["../outside"], "escape"),
    ] {
        assert!(
            SupplyRoots::open(temp.path(), &allow).is_err(),
            "{what} allow list must be rejected"
        );
    }
    assert!(
        matches!(
            SupplyRoots::open(temp.path(), &["项目/Missing"]),
            Err(SupplyError::Broker(_))
        ),
        "missing allowed directory fails closed instead of being skipped"
    );
}
