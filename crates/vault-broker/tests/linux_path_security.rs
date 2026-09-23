//! 需要真实内核路径语义的 `Linux` 文件系统攻击用例。

#![cfg(target_os = "linux")]

use std::{
    fs::{self, File},
    os::{fd::AsRawFd as _, unix::fs::symlink, unix::net::UnixListener},
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use rustix::fs::{CWD, Mode, RenameFlags, mkfifoat, renameat_with};
use studiolo_core::{PathSyntaxError, VaultPath};
use studiolo_vault_broker::{
    BrokerError, FsVaultBroker, Mutation, VaultReader as _, VaultWriter as _,
};
use tempfile::TempDir;

fn broker() -> (TempDir, TempDir, FsVaultBroker) {
    let vault = TempDir::new().expect("vault tempdir");
    let data = TempDir::new().expect("data tempdir");
    let broker = FsVaultBroker::open(vault.path(), data.path()).expect("open broker");
    (vault, data, broker)
}

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("valid test Vault path")
}

#[test]
fn symlinks_and_proc_magic_links_cannot_escape_the_vault_root() {
    let (vault, _data, broker) = broker();
    let outside = TempDir::new().expect("outside tempdir");
    fs::write(outside.path().join("secret.md"), b"outside").expect("write outside fixture");

    symlink(outside.path(), vault.path().join("ordinary-link")).expect("create symlink fixture");
    assert!(matches!(
        broker.read(&path("ordinary-link/secret.md")),
        Err(BrokerError::UnsafeNode { .. })
    ));

    let outside_directory = File::open(outside.path()).expect("open outside directory");
    let proc_magic_target = format!("/proc/self/fd/{}", outside_directory.as_raw_fd());
    symlink(proc_magic_target, vault.path().join("magic-link"))
        .expect("create proc magic-link fixture");
    assert!(matches!(
        broker.read(&path("magic-link/secret.md")),
        Err(BrokerError::UnsafeNode { .. })
    ));
}

#[test]
fn hardlinked_canonical_files_are_rejected_for_read_and_replace() {
    let (vault, _data, mut broker) = broker();
    let logical = path("notes/hardlinked.md");
    let receipt = broker
        .mutate(Mutation::create(
            logical.clone(),
            b"original".to_vec(),
            "hardlink-create",
        ))
        .expect("create canonical fixture");
    fs::hard_link(
        vault.path().join(logical.as_str()),
        vault.path().join("notes/hardlinked-copy.md"),
    )
    .expect("create hardlink fixture");

    assert!(matches!(
        broker.read(&logical),
        Err(BrokerError::UnsafeNode { .. })
    ));
    assert!(matches!(
        broker.mutate(Mutation::replace(
            logical,
            b"replacement".to_vec(),
            receipt.revision,
            "hardlink-replace",
        )),
        Err(BrokerError::UnsafeNode { .. })
    ));
}

#[test]
fn fifo_and_unix_socket_nodes_are_rejected_without_opening_them() {
    let (vault, _data, broker) = broker();
    let fifo_path = vault.path().join("incoming.fifo");
    mkfifoat(CWD, &fifo_path, Mode::RUSR | Mode::WUSR).expect("create FIFO fixture");
    let _socket =
        UnixListener::bind(vault.path().join("incoming.sock")).expect("create Unix socket fixture");

    assert!(matches!(
        broker.read(&path("incoming.fifo")),
        Err(BrokerError::UnsafeNode { .. })
    ));
    assert!(matches!(
        broker.read(&path("incoming.sock")),
        Err(BrokerError::UnsafeNode { .. })
    ));
}

#[test]
fn encoded_separators_are_rejected_before_filesystem_access() {
    for raw in [
        "notes/%2fescape.md",
        "notes/%2Fescape.md",
        "notes/%5cescape.md",
        "notes/%5Cescape.md",
        "notes/%00escape.md",
        "notes/%2e%2e/escape.md",
    ] {
        assert_eq!(
            VaultPath::parse(raw),
            Err(PathSyntaxError::ForbiddenEncoding),
            "accepted encoded path {raw:?}"
        );
    }
}

#[test]
fn case_and_unicode_ambiguous_spellings_are_rejected() {
    let (_vault, _data, mut broker) = broker();
    broker
        .mutate(Mutation::create(
            path("Notes/CAFÉ.md"),
            b"first spelling".to_vec(),
            "unicode-case-first",
        ))
        .expect("create first spelling");

    assert!(matches!(
        broker.mutate(Mutation::create(
            path("notes/café.md"),
            b"ambiguous spelling".to_vec(),
            "unicode-case-second",
        )),
        Err(BrokerError::PathCollision { .. })
    ));
    assert_eq!(
        VaultPath::parse("Notes/CAFE\u{301}.md"),
        Err(PathSyntaxError::ForbiddenEncoding)
    );
}

#[test]
fn raced_file_replacements_never_read_links_or_block_on_a_fifo() {
    for attack in ["symlink", "hardlink", "fifo"] {
        let (vault, _data, mut broker) = broker();
        broker
            .mutate(Mutation::create(
                path("slot.md"),
                b"canonical".to_vec(),
                "seed",
            ))
            .unwrap();
        let outside = TempDir::new().unwrap();
        let secret = outside.path().join("secret.md");
        fs::write(&secret, b"outside secret").unwrap();
        let alternate = vault.path().join("alternate");
        match attack {
            "symlink" => symlink(&secret, &alternate).unwrap(),
            "hardlink" => fs::hard_link(&secret, &alternate).unwrap(),
            "fifo" => mkfifoat(CWD, &alternate, Mode::RUSR | Mode::WUSR).unwrap(),
            _ => unreachable!(),
        }
        let slot = vault.path().join("slot.md");
        let stop = Arc::new(AtomicBool::new(false));
        let attack_stop = Arc::clone(&stop);
        let attacker = thread::spawn(move || {
            while !attack_stop.load(Ordering::Relaxed) {
                renameat_with(CWD, &slot, CWD, &alternate, RenameFlags::EXCHANGE).unwrap();
                thread::yield_now();
            }
        });
        let (sender, receiver) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut outside_read = false;
            for _ in 0..1500 {
                if let Ok(object) = broker.read(&path("slot.md")) {
                    outside_read |= object.content != b"canonical";
                }
            }
            sender.send(outside_read).unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(15));
        stop.store(true, Ordering::Relaxed);
        attacker.join().unwrap();
        // 失败时不要 `join` 被阻塞的读者：有界断言必须报告 `FIFO` 回归，
        // 而不是让整个测试进程挂起。
        assert!(result.is_ok(), "{attack} replacement blocked a public read");
        assert!(
            !result.unwrap(),
            "{attack} replacement exposed outside bytes"
        );
        reader.join().unwrap();
        assert_eq!(fs::read(secret).unwrap(), b"outside secret");
    }
}

#[test]
fn raced_parent_symlink_replacement_cannot_redirect_mkdir_or_writes() {
    let (vault, _data, mut broker) = broker();
    fs::create_dir(vault.path().join("moving")).unwrap();
    fs::write(vault.path().join("moving/source.md"), b"canonical").unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("source.md"), b"outside secret").unwrap();
    let alternate = vault.path().join("alternate");
    symlink(outside.path(), &alternate).unwrap();
    let moving = vault.path().join("moving");
    let stop = Arc::new(AtomicBool::new(false));
    let attack_stop = Arc::clone(&stop);
    let attacker = thread::spawn(move || {
        while !attack_stop.load(Ordering::Relaxed) {
            renameat_with(CWD, &moving, CWD, &alternate, RenameFlags::EXCHANGE).unwrap();
            thread::yield_now();
        }
    });
    let mut outside_read = false;
    for sequence in 0..200 {
        if let Ok(object) = broker.read(&path("moving/source.md")) {
            outside_read |= object.content != b"canonical";
        }
        let _ = broker.mutate(Mutation::create(
            path(&format!("moving/new-{sequence}/result.md")),
            b"result".to_vec(),
            format!("race-create-{sequence}"),
        ));
    }
    stop.store(true, Ordering::Relaxed);
    attacker.join().unwrap();
    assert!(!outside_read, "parent replacement exposed outside bytes");
    let entries = fs::read_dir(outside.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        entries,
        ["source.md"],
        "mkdir or write escaped through a racing parent"
    );
    assert_eq!(
        fs::read(outside.path().join("source.md")).unwrap(),
        b"outside secret"
    );
}

#[test]
fn concurrent_writer_admission_has_exactly_one_canonical_install() {
    let (vault, data, broker) = broker();
    drop(broker);
    let barrier = Arc::new(Barrier::new(6));
    let admitted = Arc::new(Barrier::new(6));
    let mut writers = Vec::new();
    for index in 0..6 {
        let vault_root = vault.path().to_path_buf();
        let data_root = data.path().to_path_buf();
        let start = Arc::clone(&barrier);
        let opened = Arc::clone(&admitted);
        writers.push(thread::spawn(move || {
            let content = vec![b'a' + index; 128 * 1024];
            start.wait();
            let writer = FsVaultBroker::open_existing(&vault_root, &data_root);
            // 胜出者一直持有租约，直到每个竞争中的打开操作都已返回。
            opened.wait();
            let mut writer = match writer {
                Ok(writer) => writer,
                Err(BrokerError::WriterBusy) => return None,
                Err(error) => panic!("unexpected writer admission error: {error}"),
            };
            writer
                .mutate(Mutation::create(
                    path("winner.md"),
                    content.clone(),
                    format!("creator-{index}"),
                ))
                .unwrap();
            Some(content)
        }));
    }
    let successes = writers
        .into_iter()
        .filter_map(|writer| writer.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        successes.len(),
        1,
        "the canonical root admitted more than one writer"
    );
    assert_eq!(
        fs::read(vault.path().join("winner.md")).unwrap(),
        successes[0]
    );
}
