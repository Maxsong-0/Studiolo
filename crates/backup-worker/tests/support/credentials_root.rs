//! 凭据根入口在动文件系统之前就拒掉的两种情况；真正的凭据挂载只有 systemd 造得出，放在真机上验。
use super::*;
use std::time::Duration;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

#[test]
fn another_account_or_root_is_refused_before_anything_is_opened() {
    let me = rustix::process::geteuid().as_raw();
    for uid in [0, me.wrapping_add(1)] {
        if uid == me {
            continue;
        }
        assert_eq!(
            Root::open_credentials(uid, deadline()).err(),
            Some(CredentialFault::Account),
            "{uid}"
        );
    }
}

#[test]
fn an_expired_deadline_is_reported_as_a_deadline_not_as_a_bad_directory() {
    let me = rustix::process::geteuid().as_raw();
    let fault = Root::open_credentials(me, Instant::now()).err();
    // 以 root 跑测试时先被账号那一条拒掉。
    let expected = if me == 0 {
        CredentialFault::Account
    } else {
        CredentialFault::Deadline
    };
    assert_eq!(fault, Some(expected));
}

#[test]
fn an_ordinary_directory_passes_neither_the_storage_nor_the_access_check() {
    let temp = tempfile::tempdir().unwrap();
    let fd = fs::open(temp.path(), FLAGS, Mode::empty()).unwrap();
    let stat = fs::fstat(&fd).unwrap();
    // 普通磁盘目录：不在只读内存盘上。
    assert_eq!(readonly_memory(&fd), Err(CredentialFault::Storage));
    // tempdir 是 0700、属当前账号：比两种形态都宽，别的账号号码更不行。
    assert!(!exclusive_access(
        &fd,
        Kind::Directory,
        stat.st_uid,
        stat.st_mode,
        stat.st_uid
    ));
    assert!(!exclusive_access(
        &fd,
        Kind::Directory,
        stat.st_uid,
        0o500,
        stat.st_uid.wrapping_add(1)
    ));
    // 同一个目录、没有 ACL：只有权限位正好是 0500 时才是「属主独占」那一种。
    assert!(exclusive_access(
        &fd,
        Kind::Directory,
        stat.st_uid,
        0o500,
        stat.st_uid
    ));
}
