//! 门铃存储放下即解锁，不等同一打开文件描述的其它副本关闭（BUG-041）。
//!
//! 同进程别的线程起子进程时，子进程到 exec 之前握着持锁描述的副本；这里用 `try_clone`
//! （dup，同一个打开的文件描述，不是重开）把那份副本稳定造出来，不靠抢时间窗。
use super::*;
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn dropped_doorbell_store_reopens_while_a_forked_copy_of_its_descriptor_lives() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap().join("doorbell");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let store = DoorbellStore::open(&root).unwrap();
    let inherited = store.root.root.try_clone().unwrap();
    // 副本在手、存储也开着：目录锁确实被占着，第二个打开者必须让开。
    assert!(matches!(DoorbellStore::open(&root), Err(StoreError::Busy)));
    drop(store);
    // 放下的那一刻就让位，不等「子进程」exec 关掉它那份副本。
    let again = DoorbellStore::open(&root).expect("放下后立刻能再打开");
    drop(again);
    drop(inherited);
    DoorbellStore::open(&root).expect("副本关掉后照样能打开");
}
