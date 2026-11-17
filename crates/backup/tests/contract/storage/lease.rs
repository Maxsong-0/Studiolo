//! 真进程竞争/被杀后接管，以及目录或固定锁替换时拒绝旧写者。
use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};
const CHILD: &str = "contract::storage::lease::child_process";

fn child(private: &Private, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", CHILD, "--ignored", "--nocapture"])
        .env("STUDIOLO_BACKUP_LEASE_TEST_ROOT", &private.root)
        .env("STUDIOLO_BACKUP_LEASE_TEST_MODE", mode);
    command
}
#[test]
#[ignore = "仅由租约父测试通过虚构目录和指定模式调用"]
fn child_process() {
    let root = std::env::var_os("STUDIOLO_BACKUP_LEASE_TEST_ROOT").expect("子测试专用根");
    let mode = std::env::var("STUDIOLO_BACKUP_LEASE_TEST_MODE").unwrap();
    let store = BackupStore::open(std::path::Path::new(&root), false);
    if mode == "busy" {
        assert_eq!(store.err(), Some(StoreError::Busy));
    } else {
        let store = store.unwrap();
        assert_eq!(store.load_state().unwrap(), Some(PersistedState::initial()));
        if mode == "hold" {
            println!("LEASE-READY");
            std::io::stdout().flush().unwrap();
            loop {
                std::thread::park();
            }
        }
    }
}
#[test]
fn another_process_cannot_open_until_the_original_worker_drops_its_lease() {
    let private = Private::new();
    let store = private.open(true).unwrap();
    assert_eq!(private.open(false).err(), Some(StoreError::Busy));
    // 有写者在用时再初始化：建锁之前就看到状态文件，报已存在，不去抢锁、不碰状态文件。
    assert_eq!(private.open(true).err(), Some(StoreError::AlreadyExists));
    assert!(child(&private, "busy").output().unwrap().status.success());
    drop(store);
    assert!(
        child(&private, "available")
            .output()
            .unwrap()
            .status
            .success()
    );
}
#[test]
fn abrupt_worker_death_releases_the_original_kernel_lock_without_stale_lock_removal() {
    let private = Private::new();
    drop(private.open(true).unwrap());
    let mut process = child(&private, "hold")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(process.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line.contains("LEASE-READY") {
            break;
        }
    }
    assert_eq!(private.open(false).err(), Some(StoreError::Busy));
    process.kill().unwrap();
    process.wait().unwrap();
    assert!(private.root.join("backup.lock").is_file());
    assert!(private.open(false).is_ok());
}
#[test]
fn a_replaced_root_or_lock_revokes_the_old_store_without_writing_new_state() {
    for replace_root in [false, true] {
        let private = Private::new();
        let store = private.open(true).unwrap();
        let original_state = fs::read(private.root.join(STATE_FILE)).unwrap();
        if replace_root {
            let old = private.root.with_file_name("original");
            fs::rename(&private.root, &old).unwrap();
            fs::create_dir(&private.root).unwrap();
            fs::set_permissions(&private.root, fs::Permissions::from_mode(0o700)).unwrap();
            drop(private.open(true).unwrap());
        } else {
            fs::rename(
                private.root.join("backup.lock"),
                private.root.join("old.lock"),
            )
            .unwrap();
            fs::write(private.root.join("backup.lock"), []).unwrap();
            fs::set_permissions(
                private.root.join("backup.lock"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        assert!(store.load_state().is_err());
        let mut changed = PersistedState::initial();
        changed.next_sequence = 99;
        assert!(store.save_state(&changed).is_err());
        assert_eq!(
            fs::read(private.root.join(STATE_FILE)).unwrap(),
            original_state
        );
    }
}
#[test]
fn unsafe_missing_or_nonempty_lock_cannot_be_recreated_by_normal_open() {
    for mode in 0..4 {
        let private = Private::new();
        drop(private.open(true).unwrap());
        let lock = private.root.join("backup.lock");
        match mode {
            0 => fs::remove_file(&lock).unwrap(),
            1 => {
                fs::remove_file(&lock).unwrap();
                symlink(private.root.join(STATE_FILE), &lock).unwrap();
            }
            2 => fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap(),
            _ => fs::write(&lock, b"unexpected lock body").unwrap(),
        }
        assert!(private.open(false).is_err());
        if mode == 0 {
            assert!(!lock.exists());
        }
    }
}
