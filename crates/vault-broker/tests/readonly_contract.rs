//! 零写入、显式指定根目录的 M1 读取器的公开契约。

#![cfg(unix)]

#[path = "readonly/nodes.rs"]
mod nodes;
#[path = "support/socket.rs"]
mod socket;

use std::{collections::BTreeMap, fs, io::ErrorKind, path::Path};

use studiolo_core::VaultPath;
use studiolo_vault_broker::{BrokerError, ReadOnlyVault, ScanLimits};
use tempfile::TempDir;

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("valid fixture path")
}

fn inventory(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, output: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let relative = entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .display()
                .to_string();
            if entry.file_type().unwrap().is_dir() {
                output.insert(format!("{relative}/"), Vec::new());
                visit(root, &entry.path(), output);
            } else {
                output.insert(relative, fs::read(entry.path()).unwrap());
            }
        }
    }
    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}

#[test]
fn opening_scanning_and_reading_do_not_create_or_modify_any_source_files() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("项目/甲")).unwrap();
    fs::write(root.path().join("项目/甲/INDEX.md"), "# 甲\n").unwrap();
    fs::write(root.path().join("入口.md"), "[[项目/甲/INDEX]]\n").unwrap();
    fs::write(root.path().join("app.js"), "export const app = '只读';").unwrap();
    let before = inventory(root.path());

    let reader = ReadOnlyVault::open(root.path()).unwrap();
    let batch = reader.scan_markdown(ScanLimits::default()).unwrap();
    assert_eq!(batch.notes.len(), 2);
    assert!(batch.diagnostics.is_empty());
    assert!(
        batch
            .notes
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
    );
    let asset = reader.read_file(&path("app.js"), 1024).unwrap();
    assert_eq!(asset.content, "export const app = '只读';".as_bytes());
    assert_eq!(inventory(root.path()), before);
    assert!(!root.path().join(".workbench").exists());
}

#[test]
fn opening_missing_roots_never_creates_them_and_rejects_non_directories() {
    let parent = TempDir::new().unwrap();
    let absent = parent.path().join("absent");
    assert!(matches!(
        ReadOnlyVault::open(&absent),
        Err(BrokerError::Io {
            operation: "open read-only root",
            source,
        }) if source.kind() == ErrorKind::NotFound
    ));
    assert!(!absent.exists());
    fs::write(parent.path().join("file"), b"not a directory").unwrap();
    assert!(matches!(
        ReadOnlyVault::open(&parent.path().join("file")),
        Err(BrokerError::Io {
            operation: "open read-only root",
            source,
        }) if source.kind() == ErrorKind::NotADirectory
    ));
}

#[test]
fn hidden_paths_are_never_read_or_indexed_even_when_explicitly_requested() {
    let root = TempDir::new().unwrap();
    for directory in [".git", ".obsidian", ".workbench", "project/.private"] {
        fs::create_dir_all(root.path().join(directory)).unwrap();
        fs::write(root.path().join(directory).join("secret.md"), b"hidden").unwrap();
    }
    fs::write(root.path().join(".hidden.md"), b"hidden").unwrap();
    fs::write(root.path().join("public.md"), b"visible").unwrap();
    let before = inventory(root.path());
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    for value in [".git/secret.md", "project/.private/secret.md", ".hidden.md"] {
        assert!(
            matches!(
                reader.read_file(&path(value), 1024),
                Err(BrokerError::UnsafeNode {
                    reason: "hidden path is not readable",
                    ..
                })
            ),
            "{value}"
        );
    }
    let batch = reader.scan_markdown(ScanLimits::default()).unwrap();
    assert_eq!(batch.notes.len(), 1);
    assert_eq!(batch.notes[0].path, path("public.md"));
    // 隐藏项不被读取，但仍计入枚举预算。
    assert!(matches!(
        reader.scan_markdown(ScanLimits {
            max_entries: 1,
            ..Default::default()
        }),
        Err(BrokerError::Io {
            operation: "scan entry limit exceeded",
            ..
        })
    ));
    assert_eq!(inventory(root.path()), before);
}

#[test]
fn revisions_are_stable_across_reopen_and_bound_to_logical_path_and_bytes() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("a.md"), b"same").unwrap();
    fs::write(root.path().join("b.md"), b"same").unwrap();
    let first = ReadOnlyVault::open(root.path()).unwrap();
    let original = first.read_file(&path("a.md"), 4).unwrap();
    assert_eq!(original.content, b"same");
    let second = ReadOnlyVault::open(root.path()).unwrap();
    assert_eq!(original, second.read_file(&path("a.md"), 4).unwrap());
    assert_ne!(
        original.revision,
        second.read_file(&path("b.md"), 4).unwrap().revision
    );
    fs::write(root.path().join("a.md"), b"new!").unwrap();
    assert_ne!(
        original.revision,
        second.read_file(&path("a.md"), 4).unwrap().revision
    );
}

#[test]
fn file_limit_is_diagnostic_but_total_entry_and_depth_limits_fail_the_whole_scan() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("a.md"), b"1234").unwrap();
    fs::write(root.path().join("b.md"), b"12345").unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    assert!(matches!(
        reader.read_file(&path("b.md"), 4),
        Err(BrokerError::ReadLimitExceeded { path: over, max_bytes: 4 }) if over == path("b.md")
    ));
    let batch = reader
        .scan_markdown(ScanLimits {
            max_file_bytes: 4,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(batch.notes.len(), 1);
    assert_eq!(batch.diagnostics.len(), 1);
    assert!(batch.diagnostics[0].contains("b.md"));
    assert!(!batch.diagnostics[0].contains(&root.path().display().to_string()));
    assert!(matches!(
        reader.scan_markdown(ScanLimits {
            max_total_bytes: 8,
            ..Default::default()
        }),
        Err(BrokerError::Io {
            operation: "scan total bytes limit exceeded",
            ..
        })
    ));
    assert!(matches!(
        reader.scan_markdown(ScanLimits {
            max_entries: 1,
            ..Default::default()
        }),
        Err(BrokerError::Io {
            operation: "scan entry limit exceeded",
            ..
        })
    ));
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::write(root.path().join("nested/child.md"), b"child").unwrap();
    assert!(matches!(
        reader.scan_markdown(ScanLimits {
            max_depth: 0,
            ..Default::default()
        }),
        Err(BrokerError::Io {
            operation: "scan depth limit exceeded",
            ..
        })
    ));
    assert!(
        reader
            .scan_markdown(ScanLimits {
                max_file_bytes: u64::MAX,
                ..Default::default()
            })
            .is_ok()
    );
}
