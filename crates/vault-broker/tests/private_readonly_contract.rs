//! 私有副本只读根的真实文件系统契约；测试仅写临时目录。

#![cfg(unix)]

#[path = "private_readonly/rechecks.rs"]
mod rechecks;

use std::{
    fs,
    os::unix::{
        fs::{PermissionsExt as _, symlink},
        net::UnixListener,
    },
    path::{Path, PathBuf},
};

use studiolo_core::VaultPath;
use studiolo_vault_broker::{BrokerError, ReadOnlyVault, ScanLimits};

/// 链接、特殊文件或非目录在已固定句柄下解析失败时的拒绝原因。
const NO_LINK: &str = "no-link rooted resolution failed";
/// 隐藏分量在访问文件系统前就被拒绝的原因。
const HIDDEN: &str = "hidden path is not readable";

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
    // 选定根或其祖先是链接时，逐分量 NOFOLLOW 打开在该分量处失败。
    for selected in ["alias", "alias/project/notes", "alias/project/notes/."] {
        assert!(
            matches!(
                ReadOnlyVault::open_private(&parent.join(selected)),
                Err(BrokerError::Io {
                    operation: "open private read-only ancestor",
                    ..
                })
            ),
            "{selected}"
        );
    }
    for selected in [
        Path::new("relative").to_path_buf(),
        fixture.root.join("project/../project"),
    ] {
        assert!(
            matches!(
                ReadOnlyVault::open_private(&selected),
                Err(BrokerError::Io {
                    operation: "private read-only root requires a clean absolute path",
                    ..
                })
            ),
            "{}",
            selected.display()
        );
    }
}

#[test]
fn opening_child_directory_rejects_links_and_hidden_paths() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    symlink("project", fixture.root.join("alias")).unwrap();
    directory(&fixture.root.join(".hidden"));
    // 链接无论作选定目录还是父分量，都在链接分量上拒绝；普通文件不能当目录打开。
    for (value, rejected, expected) in [
        ("alias", "alias", NO_LINK),
        ("alias/notes", "alias", NO_LINK),
        (".hidden", ".hidden", HIDDEN),
        ("asset.bin", "asset.bin", NO_LINK),
    ] {
        let result = reader.open_directory(&path(value));
        assert!(
            matches!(
                &result,
                Err(BrokerError::UnsafeNode { path: node, reason })
                    if *node == path(rejected) && *reason == expected
            ),
            "{value}: {:?}",
            result.err()
        );
    }
}

#[test]
fn private_scan_rejects_hidden_entries_instead_of_omitting_them() {
    let fixture = Fixture::new();
    let reader = fixture.open();
    file(&fixture.root.join("project/.unlisted"), "hidden content");
    assert!(matches!(
        reader.scan(ScanLimits::default()),
        Err(BrokerError::Io {
            operation: "private read-only scan rejects hidden entries",
            ..
        })
    ));
    assert!(matches!(
        reader.read_file(&path("project/.unlisted"), 100),
        Err(BrokerError::UnsafeNode { reason: HIDDEN, .. })
    ));
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
    for (value, expected) in [
        ("hard.bin", "hard-linked or unlinked file"),
        ("linked.md", NO_LINK),
        ("socket", NO_LINK),
    ] {
        let result = reader.read_file(&path(value), 100);
        assert!(
            matches!(
                &result,
                Err(BrokerError::UnsafeNode { path: node, reason })
                    if *node == path(value) && *reason == expected
            ),
            "{value}: {:?}",
            result.err()
        );
    }
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
