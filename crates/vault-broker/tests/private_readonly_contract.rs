//! 私有副本只读根的真实文件系统契约；测试仅写临时目录。

#![cfg(unix)]

use std::{
    fs,
    os::unix::{
        fs::{PermissionsExt as _, symlink},
        net::UnixListener,
    },
    path::{Path, PathBuf},
};

use studiolo_core::VaultPath;
use studiolo_vault_broker::{ReadOnlyVault, ScanLimits};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        // macOS 的 /var、/tmp 可为系统链接；测试给显式无链接绝对路径。
        let root = temp.path().canonicalize().unwrap().join("replica");
        directory(&root);
        directory(&root.join("project"));
        directory(&root.join("project/notes"));
        file(&root.join("project/notes/note.md"), "private note");
        file(&root.join("asset.bin"), "private asset");
        Self { _temp: temp, root }
    }

    fn open(&self) -> ReadOnlyVault {
        ReadOnlyVault::open_private(&self.root).unwrap()
    }
}

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

fn directory(path: &Path) {
    fs::create_dir(path).unwrap();
    permissions(path, 0o700);
}

fn file(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    permissions(path, 0o600);
}

fn permissions(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn private_root_reads_and_scans_only_safe_modes() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    let note = reader
        .read_file(&path("project/notes/note.md"), 100)
        .unwrap();
    assert_eq!(note.content, b"private note");
    let batch = reader.scan(ScanLimits::default()).unwrap();
    assert!(batch.diagnostics.is_empty());
    assert_eq!(batch.entries.len(), 4);
    assert_eq!(batch.notes.len(), 1);
}

#[test]
fn private_root_rejects_links_in_selected_root_and_intermediate_ancestors() {
    let fixture = Fixture::new();
    let parent = fixture.root.parent().unwrap();
    symlink(&fixture.root, parent.join("alias")).unwrap();
    assert!(ReadOnlyVault::open_private(&parent.join("alias")).is_err());
    assert!(ReadOnlyVault::open_private(&parent.join("alias/project/notes")).is_err());
    assert!(ReadOnlyVault::open_private(&parent.join("alias/project/notes/.")).is_err());
    assert!(ReadOnlyVault::open_private(Path::new("relative")).is_err());
    assert!(ReadOnlyVault::open_private(&fixture.root.join("project/../project")).is_err());
}

#[test]
fn root_permissions_are_rechecked_after_open() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    permissions(&fixture.root, 0o750);
    assert!(ReadOnlyVault::open_private(&fixture.root).is_err());
    assert!(reader.read_file(&path("asset.bin"), 100).is_err());
    assert!(reader.open_directory(&path("project")).is_err());
    assert!(reader.scan(ScanLimits::default()).is_err());
}

#[test]
fn intermediate_directory_permissions_are_checked_for_reads_and_scan() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    permissions(&fixture.root.join("project"), 0o755);
    assert!(
        reader
            .read_file(&path("project/notes/note.md"), 100)
            .is_err()
    );
    assert!(reader.open_directory(&path("project/notes")).is_err());
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
    assert!(
        reader
            .read_file(&path("project/notes/note.md"), 100)
            .is_err()
    );
    assert!(reader.read_file(&path("asset.bin"), 100).is_err());
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
    assert!(child.read_file(&path("note.md"), 100).is_err());
    permissions(&moved.join("project/notes"), 0o750);
    assert!(child.scan(ScanLimits::default()).is_err());
}

#[test]
fn opening_child_directory_rejects_links_and_hidden_paths() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    symlink("project", fixture.root.join("alias")).unwrap();
    assert!(reader.open_directory(&path("alias")).is_err());
    assert!(reader.open_directory(&path("alias/notes")).is_err());
    directory(&fixture.root.join(".hidden"));
    assert!(reader.open_directory(&path(".hidden")).is_err());
    assert!(reader.open_directory(&path("asset.bin")).is_err());
}

#[test]
fn private_scan_rejects_hidden_entries_instead_of_omitting_them() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    file(&fixture.root.join("project/.unlisted"), "hidden content");
    assert!(reader.scan(ScanLimits::default()).is_err());
    assert!(reader.read_file(&path("project/.unlisted"), 100).is_err());
}

#[test]
fn links_and_special_files_remain_rejected() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    fs::hard_link(
        fixture.root.join("asset.bin"),
        fixture.root.join("hard.bin"),
    )
    .unwrap();
    symlink("project/notes/note.md", fixture.root.join("linked.md")).unwrap();
    let _socket = UnixListener::bind(fixture.root.join("socket")).unwrap();
    assert!(reader.read_file(&path("hard.bin"), 100).is_err());
    assert!(reader.read_file(&path("linked.md"), 100).is_err());
    assert!(reader.read_file(&path("socket"), 100).is_err());
    let batch = reader.scan(ScanLimits::default()).unwrap();
    assert_eq!(batch.diagnostics.len(), 4);
}

#[test]
fn default_readonly_mode_keeps_existing_permissions_and_hidden_semantics() {
    let fixture = Fixture::new();
    permissions(&fixture.root, 0o755);
    permissions(&fixture.root.join("project"), 0o755);
    permissions(&fixture.root.join("project/notes/note.md"), 0o644);
    file(&fixture.root.join(".ignored"), "hidden");
    let reader = ReadOnlyVault::open(&fixture.root).unwrap();
    assert_eq!(
        reader
            .read_file(&path("project/notes/note.md"), 100)
            .unwrap()
            .content,
        b"private note"
    );
    assert!(
        reader
            .scan(ScanLimits::default())
            .unwrap()
            .diagnostics
            .is_empty()
    );
    let child = reader.open_directory(&path("project/notes")).unwrap();
    assert!(child.read_file(&path("note.md"), 100).is_ok());
}
