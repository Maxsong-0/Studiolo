//! 目录身份与附件元信息的竞态回归；仅限临时虚构目录。
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::symlink,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
use studiolo_vault_broker::{ReadEntryKind, ReadOnlyVault, ScanLimits};
use tempfile::TempDir;

#[test]
fn renamed_selected_root_remains_pinned_and_does_not_switch_to_a_replacement() {
    let parent = TempDir::new().unwrap();
    let selected = parent.path().join("selected");
    fs::create_dir(&selected).unwrap();
    fs::write(selected.join("original.md"), b"original").unwrap();
    let reader = ReadOnlyVault::open(&selected).unwrap();
    fs::rename(&selected, parent.path().join("pinned-original")).unwrap();
    fs::create_dir(&selected).unwrap();
    fs::write(selected.join("replacement.md"), b"must not switch").unwrap();
    let batch = reader.scan(ScanLimits::default()).unwrap();
    assert!(batch.diagnostics.is_empty());
    assert_eq!(batch.notes.len(), 1);
    assert_eq!(batch.notes[0].content, b"original");
    assert_eq!(batch.entries[0].path.as_str(), "original.md");
}

#[test]
fn racing_directory_rename_never_scans_an_outside_link_target() {
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let moving = root.path().join("moving");
    let parked = root.path().join(".parked");
    fs::create_dir(&moving).unwrap();
    fs::write(moving.join("note.md"), b"safe").unwrap();
    fs::write(moving.join("image.bin"), b"safe").unwrap();
    fs::write(outside.path().join("note.md"), b"outside secret").unwrap();
    fs::write(outside.path().join("outside-only.bin"), [0_u8; 999]).unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = Arc::clone(&stop);
    let outside_path = outside.path().to_owned();
    let writer = thread::spawn(move || {
        while !writer_stop.load(Ordering::Acquire) {
            fs::rename(&moving, &parked).unwrap();
            symlink(&outside_path, &moving).unwrap();
            fs::remove_file(&moving).unwrap();
            fs::rename(&parked, &moving).unwrap();
        }
    });
    let mut escaped = false;
    for _ in 0..128 {
        if let Ok(batch) = reader.scan(ScanLimits::default()) {
            escaped |= batch.notes.iter().any(|note| note.content != b"safe");
            escaped |= batch
                .entries
                .iter()
                .any(|entry| entry.path.as_str().contains("outside-only"));
        }
    }
    stop.store(true, Ordering::Release);
    writer.join().unwrap();
    assert!(!escaped, "成功返回的扫描不能包含外部目录条目或正文");
}

#[test]
fn racing_attachment_replacement_does_not_expose_symlink_target_metadata() {
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let target = outside.path().join("secret.bin");
    fs::write(&target, [0_u8; 999]).unwrap();
    fs::write(root.path().join("item.bin"), b"safe").unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = Arc::clone(&stop);
    let directory = root.path().to_owned();
    let writer = thread::spawn(move || {
        while !writer_stop.load(Ordering::Acquire) {
            let replacement = directory.join(".replacement");
            symlink(&target, &replacement).unwrap();
            fs::rename(&replacement, directory.join("item.bin")).unwrap();
            fs::write(&replacement, b"safe").unwrap();
            fs::rename(&replacement, directory.join("item.bin")).unwrap();
        }
    });
    let mut escaped = false;
    for _ in 0..256 {
        if let Ok(batch) = reader.scan(ScanLimits::default()) {
            escaped |= batch.entries.iter().any(|entry| {
                entry.kind == ReadEntryKind::Attachment && entry.size_bytes != Some(4)
            });
        }
    }
    stop.store(true, Ordering::Release);
    writer.join().unwrap();
    assert!(!escaped, "附件元信息不能跟随链接泄漏外部文件大小");
}
