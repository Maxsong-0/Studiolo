//! 作业目录租约只能操作固定空锁，路径模块没有创建/覆写能力。
use super::super::authority_tokens::checked_closed;
use std::path::Path;
pub(super) fn remainder(path: &Path, source: &str) -> String {
    if path.ends_with("crates/backup/src/store/storage/paths.rs") {
        return closed_io(
            source,
            &[
                "OFlags::RDONLY|OFlags::DIRECTORY|OFlags::NOFOLLOW|OFlags::NONBLOCK|OFlags::CLOEXEC",
                "ifgetuid()!=geteuid()||getgid()!=getegid()",
                "stat.st_nlink!=1",
                "_=>returnErr(StoreError::UnsafeStorage)",
            ],
            &[
                ("rustix::fs::{FileType,Mode,OFlags,fstat,open,openat}", 1),
                ("open(\"/\",flags,Mode::empty())", 1),
                ("openat(&directory,name,flags,Mode::empty())", 1),
                ("fstat(directory)", 1),
                ("fstat(file)", 1),
            ],
        );
    }
    if path.ends_with("crates/backup/src/store/storage/lease.rs") {
        return closed_io(
            source,
            &[
                "constLOCK:&str=\"backup.lock\";",
                "letflags=OFlags::RDWR|OFlags::NOFOLLOW|OFlags::CLOEXEC|OFlags::NONBLOCK;",
                "check_regular(&lock,owner,0)?",
                "identity(&current)?!=self.root||identity(root)?!=self.root",
                "identity(&current_lock)?!=identity(&self.lock)?",
                "Err(rustix::io::Errno::EXIST)=>returnErr(StoreError::AlreadyExists)",
            ],
            &[
                (
                    "rustix::{fd::OwnedFd,fs::{FlockOperation,Mode,OFlags,flock,fstat,fsync,openat}}",
                    1,
                ),
                ("flock(&lock,FlockOperation::NonBlockingLockExclusive)", 1),
                ("flock(&self.lock,FlockOperation::Unlock)", 1),
                (
                    "openat(root,LOCK,flags|OFlags::CREATE|OFlags::EXCL,Mode::RUSR|Mode::WUSR)",
                    1,
                ),
                ("openat(root,LOCK,flags,Mode::empty())", 1),
                (
                    "openat(root,LOCK,OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::CLOEXEC|OFlags::NONBLOCK,Mode::empty())",
                    1,
                ),
                ("fsync(&fd)", 1),
                ("fsync(root)", 1),
                ("fstat(fd)", 1),
            ],
        );
    }
    source.to_owned()
}
fn closed_io(source: &str, anchors: &[&str], calls: &[(&str, usize)]) -> String {
    let remaining = checked_closed(source, anchors, calls);
    if ["openat(", "fsync(", "fstat("]
        .iter()
        .any(|call| remaining.contains(call))
    {
        super::super::authority_tokens::compact(source)
    } else {
        remaining
    }
}
