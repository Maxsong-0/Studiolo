//! 租约放下即解锁，不等同一打开文件描述的其它副本关闭（BUG-041）。
//!
//! 同进程别的线程起子进程时，子进程到 exec 之前握着持锁描述的副本；这里用 `try_clone`
//! （dup，同一个打开的文件描述，不是重开）把那份副本稳定造出来，不靠抢时间窗。
use super::*;
use std::{fs, os::unix::fs::PermissionsExt};

fn parent() -> (tempfile::TempDir, Directory) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("parent");
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let parent = Directory::open(&path, true).unwrap();
    (temp, parent)
}

#[test]
fn dropped_lease_is_reacquirable_while_a_forked_copy_of_its_descriptor_lives() {
    let (_temp, parent) = parent();
    let lease = Lease::acquire(&parent).unwrap();
    let inherited = lease.fd.try_clone().unwrap();
    // 副本在手、租约也在手：锁确实被占着，重拿必须让开。
    assert_eq!(Lease::existing(&parent).err(), Some(StagingError::Busy));
    drop(lease);
    // 租约放下的那一刻就结束，不等「子进程」exec 关掉它那份副本。
    let again = Lease::existing(&parent).expect("放下后立刻能重拿");
    drop(again);
    drop(inherited);
    Lease::existing(&parent).expect("副本关掉后照样能拿");
}
