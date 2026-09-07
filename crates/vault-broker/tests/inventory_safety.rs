//! 不安全附件、文件名冲突与元信息权限的公开契约。
#![cfg(unix)]

use std::fs;
use studiolo_core::VaultPath;
use studiolo_vault_broker::{ReadOnlyVault, ScanLimits};
use tempfile::TempDir;

#[test]
fn unsafe_attachments_are_diagnostic_instead_of_silently_missing() {
    use std::os::unix::{fs::symlink, net::UnixListener};
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret.bin"), b"not exposed").unwrap();
    symlink(
        outside.path().join("secret.bin"),
        root.path().join("link.bin"),
    )
    .unwrap();
    symlink(outside.path(), root.path().join("linked-dir")).unwrap();
    fs::write(root.path().join("hard.pdf"), b"shared").unwrap();
    fs::hard_link(root.path().join("hard.pdf"), root.path().join("other.pdf")).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(root.path().join("pipe.bin"))
            .status()
            .unwrap()
            .success()
    );
    let socket = UnixListener::bind(root.path().join("socket.bin")).ok();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    let batch = reader.scan(ScanLimits::default()).unwrap();
    assert!(batch.entries.is_empty());
    assert!(batch.notes.is_empty());
    assert_eq!(
        batch.diagnostics.len(),
        if socket.is_some() { 6 } else { 5 }
    );
    assert!(
        batch
            .diagnostics
            .iter()
            .all(|message| !message.contains("not exposed")
                && !message.contains(&root.path().display().to_string()))
    );
}

#[test]
fn noncanonical_and_case_colliding_attachments_or_directories_are_not_selected() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("Name.pdf"), b"first").unwrap();
    fs::write(root.path().join("name.pdf"), b"second").unwrap();
    fs::create_dir_all(root.path().join("Folder")).unwrap();
    fs::create_dir_all(root.path().join("folder")).unwrap();
    let case_sensitive = fs::read_dir(root.path()).unwrap().count() == 4;
    fs::write(root.path().join("cafe\u{301}.bin"), b"noncanonical").unwrap();
    let batch = ReadOnlyVault::open(root.path())
        .unwrap()
        .scan(ScanLimits::default())
        .unwrap();
    assert!(
        batch
            .entries
            .iter()
            .all(|entry| !entry.path.as_str().contains('\u{301}'))
    );
    assert!(!batch.diagnostics.is_empty());
    if case_sensitive {
        assert!(batch.entries.is_empty());
        assert_eq!(batch.diagnostics.len(), 5);
    }
    assert!(VaultPath::parse("cafe\u{301}.bin").is_err());
}

#[test]
fn metadata_only_scan_can_inspect_a_file_without_content_read_permission() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let file = root.path().join("sealed.bin");
    fs::write(&file, b"sealed attachment").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    let result = ReadOnlyVault::open(root.path())
        .unwrap()
        .scan(ScanLimits::default());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let batch = result.unwrap();
    assert!(batch.diagnostics.is_empty());
    assert_eq!(batch.entries[0].size_bytes, Some(17));
    assert!(batch.notes.is_empty());
}
