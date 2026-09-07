//! Public contract for the zero-write, explicitly rooted M1 reader.

#![cfg(unix)]

use std::{collections::BTreeMap, fs, path::Path};

use studiolo_core::VaultPath;
use studiolo_vault_broker::{ReadOnlyVault, ScanLimits};
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
    assert!(ReadOnlyVault::open(&absent).is_err());
    assert!(!absent.exists());
    fs::write(parent.path().join("file"), b"not a directory").unwrap();
    assert!(ReadOnlyVault::open(&parent.path().join("file")).is_err());
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
        assert!(reader.read_file(&path(value), 1024).is_err());
    }
    let batch = reader.scan_markdown(ScanLimits::default()).unwrap();
    assert_eq!(batch.notes.len(), 1);
    assert_eq!(batch.notes[0].path, path("public.md"));
    assert!(
        reader
            .scan_markdown(ScanLimits {
                max_entries: 1,
                ..Default::default()
            })
            .is_err()
    );
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
    assert!(reader.read_file(&path("b.md"), 4).is_err());
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
    assert!(
        reader
            .scan_markdown(ScanLimits {
                max_total_bytes: 8,
                ..Default::default()
            })
            .is_err()
    );
    assert!(
        reader
            .scan_markdown(ScanLimits {
                max_entries: 1,
                ..Default::default()
            })
            .is_err()
    );
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::write(root.path().join("nested/child.md"), b"child").unwrap();
    assert!(
        reader
            .scan_markdown(ScanLimits {
                max_depth: 0,
                ..Default::default()
            })
            .is_err()
    );
    assert!(
        reader
            .scan_markdown(ScanLimits {
                max_file_bytes: u64::MAX,
                ..Default::default()
            })
            .is_ok()
    );
}

#[test]
fn symlinked_roots_parents_files_hardlinks_and_special_nodes_are_rejected() {
    use std::os::unix::{fs::symlink, net::UnixListener};

    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret.md"), b"outside secret").unwrap();
    symlink(outside.path(), root.path().join("escape")).unwrap();
    symlink(
        outside.path().join("secret.md"),
        root.path().join("link.md"),
    )
    .unwrap();
    assert!(ReadOnlyVault::open(&root.path().join("escape")).is_err());
    assert!(ReadOnlyVault::open(&root.path().join("escape/")).is_err());
    assert!(ReadOnlyVault::open(&root.path().join("escape/.")).is_err());
    fs::write(root.path().join("hard.md"), b"shared").unwrap();
    fs::hard_link(root.path().join("hard.md"), root.path().join("other.md")).unwrap();
    // rustix 的 mkfifoat 不是 macOS API；测试通过系统工具创建临时 FIFO。
    assert!(
        std::process::Command::new("mkfifo")
            .arg(root.path().join("pipe.md"))
            .status()
            .unwrap()
            .success()
    );
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    for value in [
        "escape/secret.md",
        "link.md",
        "hard.md",
        "other.md",
        "pipe.md",
    ] {
        assert!(reader.read_file(&path(value), 1024).is_err(), "{value}");
    }
    if let Ok(_socket) = UnixListener::bind(root.path().join("socket.md")) {
        assert!(reader.read_file(&path("socket.md"), 1024).is_err());
    }
    let batch = reader.scan_markdown(ScanLimits::default()).unwrap();
    assert!(batch.notes.is_empty());
    assert!(batch.diagnostics.len() >= 5);
    assert!(
        batch
            .diagnostics
            .iter()
            .all(|message| !message.contains("outside secret"))
    );
}

#[test]
fn case_collisions_and_noncanonical_names_do_not_get_silently_selected() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("Name.md"), b"first").unwrap();
    fs::write(root.path().join("name.md"), b"second").unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    if fs::read_dir(root.path()).unwrap().count() == 2 {
        assert!(reader.read_file(&path("Name.md"), 1024).is_err());
        assert!(
            reader
                .scan_markdown(ScanLimits::default())
                .unwrap()
                .notes
                .is_empty()
        );
    }
    assert!(VaultPath::parse("cafe\u{301}.md").is_err());
}

#[test]
fn racing_file_replacement_never_follows_an_outside_symlink() {
    use std::{
        os::unix::fs::symlink,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };

    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let secret = outside.path().join("secret.md");
    fs::write(&secret, b"outside secret").unwrap();
    fs::write(root.path().join("note.md"), b"safe").unwrap();
    let reader = ReadOnlyVault::open(root.path()).unwrap();
    let running = Arc::new(AtomicBool::new(true));
    let writer_running = Arc::clone(&running);
    let directory = root.path().to_path_buf();
    let writer = thread::spawn(move || {
        while writer_running.load(Ordering::Acquire) {
            let temp = directory.join("replacement");
            symlink(&secret, &temp).unwrap();
            fs::rename(&temp, directory.join("note.md")).unwrap();
            fs::write(&temp, b"safe").unwrap();
            fs::rename(&temp, directory.join("note.md")).unwrap();
        }
    });
    let mut escaped = false;
    for _ in 0..128 {
        if let Ok(note) = reader.read_file(&path("note.md"), 1024) {
            escaped |= note.content != b"safe";
        }
    }
    running.store(false, Ordering::Release);
    writer.join().unwrap();
    assert!(
        !escaped,
        "a successful read must never contain outside bytes"
    );
}
