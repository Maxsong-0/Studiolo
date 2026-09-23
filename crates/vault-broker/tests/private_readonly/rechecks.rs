//! 私有模式在打开后仍逐次复核根、中间目录、文件与子目录句柄的属主和权限位。

use std::fs;

use studiolo_vault_broker::{BrokerError, ReadOnlyVault, ScanLimits};

use super::{Fixture, directory, file, path, permissions};

/// 私有模式属主或权限不符时的稳定操作标签。
const UNSAFE_MODE: &str = "private read-only node has unsafe ownership or permissions";

#[test]
fn root_permissions_are_rechecked_after_open() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    permissions(&fixture.root, 0o750);
    assert!(matches!(
        ReadOnlyVault::open_private(&fixture.root),
        Err(BrokerError::Io {
            operation: UNSAFE_MODE,
            ..
        })
    ));
    assert!(matches!(
        reader.read_file(&path("asset.bin"), 100),
        Err(BrokerError::Io {
            operation: UNSAFE_MODE,
            ..
        })
    ));
    assert!(matches!(
        reader.open_directory(&path("project")),
        Err(BrokerError::Io {
            operation: UNSAFE_MODE,
            ..
        })
    ));
    assert!(matches!(
        reader.scan(ScanLimits::default()),
        Err(BrokerError::Io {
            operation: UNSAFE_MODE,
            ..
        })
    ));
}

#[test]
fn intermediate_directory_permissions_are_checked_for_reads_and_scan() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    permissions(&fixture.root.join("project"), 0o755);
    assert!(matches!(
        reader.read_file(&path("project/notes/note.md"), 100),
        Err(BrokerError::Io {
            operation: UNSAFE_MODE,
            ..
        })
    ));
    assert!(matches!(
        reader.open_directory(&path("project/notes")),
        Err(BrokerError::Io {
            operation: UNSAFE_MODE,
            ..
        })
    ));
    let batch = reader.scan(ScanLimits::default()).unwrap();
    assert!(!batch.diagnostics.is_empty());
    assert!(batch.notes.is_empty());
}

#[test]
fn file_permissions_are_rechecked_for_markdown_and_metadata_only_attachments() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    permissions(&fixture.root.join("project/notes/note.md"), 0o644);
    permissions(&fixture.root.join("asset.bin"), 0o640);
    for value in ["project/notes/note.md", "asset.bin"] {
        assert!(
            matches!(
                reader.read_file(&path(value), 100),
                Err(BrokerError::Io {
                    operation: UNSAFE_MODE,
                    ..
                })
            ),
            "{value}"
        );
    }
    let batch = reader.scan(ScanLimits::default()).unwrap();
    assert_eq!(batch.diagnostics.len(), 2);
    assert!(batch.notes.is_empty());
}

#[test]
fn child_directory_is_handle_rooted_and_inherits_private_mode() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    let moved = fixture.root.with_file_name("retained");
    fs::rename(&fixture.root, &moved).unwrap();
    directory(&fixture.root);
    directory(&fixture.root.join("project"));
    file(&fixture.root.join("project/replacement.md"), "replacement");
    let child = reader.open_directory(&path("project/notes")).unwrap();
    assert_eq!(
        child.read_file(&path("note.md"), 100).unwrap().content,
        b"private note"
    );
    permissions(&moved.join("project/notes/note.md"), 0o644);
    assert!(matches!(
        child.read_file(&path("note.md"), 100),
        Err(BrokerError::Io {
            operation: UNSAFE_MODE,
            ..
        })
    ));
    permissions(&moved.join("project/notes"), 0o750);
    assert!(matches!(
        child.scan(ScanLimits::default()),
        Err(BrokerError::Io {
            operation: UNSAFE_MODE,
            ..
        })
    ));
}
