//! 不安全附件、文件名冲突与元信息权限的公开契约。
#![cfg(unix)]

#[path = "support/socket.rs"]
mod socket;

use std::fs;
use studiolo_core::{PathSyntaxError, VaultPath};
use studiolo_vault_broker::{ReadOnlyVault, ScanLimits};
use tempfile::TempDir;

#[test]
fn unsafe_attachments_are_diagnostic_instead_of_silently_missing() {
    use std::os::unix::fs::symlink;
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
    // 路径过长建不成时由夹具打印跳过原因，诊断条数随之少一条。
    let listener = socket::bind_fixture(&root.path().join("socket.bin"));
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    let batch = reader.scan(ScanLimits::default()).unwrap();
    assert!(batch.entries.is_empty());
    assert!(batch.notes.is_empty());
    assert_eq!(
        batch.diagnostics.len(),
        if listener.is_some() { 6 } else { 5 }
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
    let mut names = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    let case_sensitive = names.len() == 4;
    // 分支自证：卷是否区分大小写决定走哪一支；打印判据原始输入（实际目录项）与所选分支。
    eprintln!(
        "大小写碰撞·分支自证：目录项 = {names:?}，区分大小写 = {case_sensitive}（{}）",
        if case_sensitive {
            "同名异写的文件与目录都并存，全部按冲突诊断"
        } else {
            "第二次创建落到同一节点，只剩磁盘原拼写"
        }
    );
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
    let entries = batch
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();
    eprintln!(
        "大小写碰撞·扫描结果：entries = {entries:?}，diagnostics = {:?}",
        batch.diagnostics
    );
    if case_sensitive {
        assert!(entries.is_empty());
        assert_eq!(batch.diagnostics.len(), 5);
    } else {
        eprintln!(
            "跳过：临时卷不区分大小写，无法构造并存的同名异写；改验只剩的磁盘原拼写被正常收录"
        );
        assert_eq!(names, ["Folder", "Name.pdf"]);
        assert_eq!(entries, ["Folder", "Name.pdf"]);
        // 只有分解形式的名称按非规范项诊断。
        assert_eq!(batch.diagnostics.len(), 1);
        assert!(batch.diagnostics[0].contains("noncanonical"));
    }
    assert_eq!(
        VaultPath::parse("cafe\u{301}.bin"),
        Err(PathSyntaxError::ForbiddenEncoding)
    );
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
