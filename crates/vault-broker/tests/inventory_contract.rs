//! 文件树与附件元信息的公开只读 Interface；仅使用临时虚构夹具。
#![cfg(unix)]

use std::{fs, path::Path};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{BrokerError, ReadEntryKind, ReadOnlyVault, ScanLimits};
use tempfile::TempDir;

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

#[test]
fn scan_preserves_empty_and_attachment_only_directories_without_reading_attachment_bytes() {
    let root = TempDir::new().unwrap();
    for name in ["空目录", "资料/仅附件", ".obsidian", "资料/.私有"] {
        fs::create_dir_all(root.path().join(name)).unwrap();
    }
    fs::write(root.path().join("入口.MD"), "# 示例\n").unwrap();
    fs::write(root.path().join("资料/仅附件/二进制.bin"), [0xff, 0, 0xfe]).unwrap();
    fs::write(root.path().join(".obsidian/隐藏.md"), "hidden").unwrap();
    fs::write(root.path().join("资料/.私有/隐藏.pdf"), "hidden").unwrap();
    let large = fs::File::create(root.path().join("资料/巨大.dat")).unwrap();
    large.set_len(1024 * 1024 * 1024 * 1024).unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    let batch = reader
        .scan(ScanLimits {
            max_file_bytes: 32,
            max_total_bytes: 32,
            ..Default::default()
        })
        .unwrap();
    assert!(batch.diagnostics.is_empty());
    assert_eq!(batch.notes.len(), 1);
    assert_eq!(batch.notes[0].content, "# 示例\n".as_bytes());
    assert_eq!(batch.entries.len(), 6);
    assert!(
        batch
            .entries
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
    );
    for name in ["空目录", "资料", "资料/仅附件"] {
        let entry = batch
            .entries
            .iter()
            .find(|entry| entry.path == path(name))
            .unwrap();
        assert_eq!(entry.kind, ReadEntryKind::Directory);
        assert_eq!(entry.size_bytes, None);
    }
    for (name, size, kind) in [
        ("入口.MD", "# 示例\n".len() as u64, ReadEntryKind::Markdown),
        ("资料/仅附件/二进制.bin", 3, ReadEntryKind::Attachment),
        (
            "资料/巨大.dat",
            1024 * 1024 * 1024 * 1024,
            ReadEntryKind::Attachment,
        ),
    ] {
        let entry = batch
            .entries
            .iter()
            .find(|entry| entry.path == path(name))
            .unwrap();
        assert_eq!(entry.kind, kind);
        assert_eq!(entry.size_bytes, Some(size));
    }
    assert_eq!(reader.scan_markdown(ScanLimits::default()).unwrap(), batch);
    assert!(!root.path().join(".workbench").exists());
    assert_eq!(
        fs::read(root.path().join("资料/仅附件/二进制.bin")).unwrap(),
        [0xff, 0, 0xfe]
    );
    assert_eq!(
        fs::metadata(root.path().join("资料/巨大.dat"))
            .unwrap()
            .len(),
        1024 * 1024 * 1024 * 1024
    );
}

#[test]
fn attachment_and_hidden_entries_consume_the_complete_scan_budget() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join(".hidden"), b"hidden").unwrap();
    fs::write(root.path().join("attachment.bin"), b"data").unwrap();
    fs::create_dir(root.path().join("empty")).unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    // 隐藏项与附件同样占用枚举预算。
    assert!(matches!(
        reader.scan(ScanLimits {
            max_entries: 2,
            ..Default::default()
        }),
        Err(BrokerError::Io {
            operation: "scan entry limit exceeded",
            ..
        })
    ));
    assert!(matches!(
        reader.scan(ScanLimits {
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
            .scan(ScanLimits {
                max_entries: 3,
                max_total_bytes: 0,
                ..Default::default()
            })
            .is_ok()
    );
}

#[test]
fn oversized_markdown_never_appears_as_a_successful_tree_document() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("big.md"), b"too large").unwrap();
    fs::write(root.path().join("small.bin"), b"raw").unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    let batch = reader
        .scan(ScanLimits {
            max_file_bytes: 1,
            ..Default::default()
        })
        .unwrap();
    assert!(batch.notes.is_empty());
    assert_eq!(batch.entries.len(), 1);
    assert_eq!(batch.entries[0].path, path("small.bin"));
    assert_eq!(batch.diagnostics.len(), 1);
}

fn snapshot_names(root: &Path) -> Vec<String> {
    let mut result = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    result.sort();
    result
}

#[test]
fn a_wide_attachment_directory_is_complete_in_stable_order_without_mutations() {
    let root = TempDir::new().unwrap();
    for index in 0..10_000 {
        fs::write(root.path().join(format!("item-{index:05}.bin")), []).unwrap();
    }
    let before = snapshot_names(root.path());
    let batch = ReadOnlyVault::open(root.path())
        .unwrap()
        .scan(ScanLimits::default())
        .unwrap();
    assert!(batch.diagnostics.is_empty());
    assert_eq!(batch.entries.len(), 10_000);
    assert!(batch.notes.is_empty());
    assert_eq!(before, snapshot_names(root.path()));
}
