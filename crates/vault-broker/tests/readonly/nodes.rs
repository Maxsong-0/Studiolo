//! 链接、硬链接、特殊节点与大小写别名一律拒绝；环境决定的分支须打印判据与所选分支。

use std::fs;

use studiolo_core::{PathSyntaxError, VaultPath};
use studiolo_vault_broker::{BrokerError, ReadOnlyVault, ScanLimits};
use tempfile::TempDir;

use super::{path, socket};

#[test]
fn symlinked_roots_parents_files_hardlinks_and_special_nodes_are_rejected() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret.md"), b"outside secret").unwrap();
    symlink(outside.path(), root.path().join("escape")).unwrap();
    symlink(
        outside.path().join("secret.md"),
        root.path().join("link.md"),
    )
    .unwrap();
    // 结尾斜杠与 `/.` 都被规整掉，不能借此绕过最后一级 NOFOLLOW。
    for selected in ["escape", "escape/", "escape/."] {
        assert!(
            matches!(
                ReadOnlyVault::open(&root.path().join(selected)),
                Err(BrokerError::Io {
                    operation: "open read-only root",
                    ..
                })
            ),
            "{selected}"
        );
    }
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
    // 父目录是链接时拒绝落在父分量上，其余节点按自身类型拒绝。
    for (value, rejected, expected) in [
        (
            "escape/secret.md",
            "escape",
            "no-link rooted resolution failed",
        ),
        ("link.md", "link.md", "no-link rooted resolution failed"),
        ("hard.md", "hard.md", "hard-linked or unlinked file"),
        ("other.md", "other.md", "hard-linked or unlinked file"),
        ("pipe.md", "pipe.md", "not a regular file"),
    ] {
        let result = reader.read_file(&path(value), 1024);
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
    // 路径过长建不成时由夹具打印跳过原因；建成时 socket 必须按不可打开的节点拒绝。
    let listener = socket::bind_fixture(&root.path().join("socket.md"));
    if listener.is_some() {
        assert!(matches!(
            reader.read_file(&path("socket.md"), 1024),
            Err(BrokerError::UnsafeNode {
                reason: "no-link rooted resolution failed",
                ..
            })
        ));
    }
    let batch = reader.scan_markdown(ScanLimits::default()).unwrap();
    assert!(batch.notes.is_empty());
    // escape、link.md、hard.md、other.md、pipe.md 各一条，socket 建成时再加一条。
    assert_eq!(
        batch.diagnostics.len(),
        if listener.is_some() { 6 } else { 5 },
        "{:?}",
        batch.diagnostics
    );
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
    let mut names = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    let case_sensitive = names.len() == 2;
    // 分支自证：卷是否区分大小写决定走哪一支；打印判据原始输入（实际目录项）与所选分支。
    eprintln!(
        "大小写碰撞·分支自证：目录项 = {names:?}，区分大小写 = {case_sensitive}（{}）",
        if case_sensitive {
            "两个拼写并存，二者都拒绝"
        } else {
            "第二次写入落到同一文件，只认磁盘原拼写"
        }
    );
    let collision = |value: &str| {
        matches!(
            reader.read_file(&path(value), 1024),
            Err(BrokerError::UnsafeNode {
                reason: "case-colliding filesystem name",
                ..
            })
        )
    };
    // 与磁盘拼写只差大小写的请求在两类卷上都不能被静默选中。
    assert!(collision("name.md"));
    if case_sensitive {
        assert!(collision("Name.md"));
        assert!(
            reader
                .scan_markdown(ScanLimits::default())
                .unwrap()
                .notes
                .is_empty()
        );
    } else {
        eprintln!("跳过：临时卷不区分大小写，无法构造并存的两个拼写；改验唯一的磁盘原拼写");
        assert_eq!(names, ["Name.md"]);
        assert_eq!(
            reader.read_file(&path("Name.md"), 1024).unwrap().content,
            b"second"
        );
        let notes = reader.scan_markdown(ScanLimits::default()).unwrap().notes;
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].path, path("Name.md"));
    }
    assert_eq!(
        VaultPath::parse("cafe\u{301}.md"),
        Err(PathSyntaxError::ForbiddenEncoding)
    );
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
