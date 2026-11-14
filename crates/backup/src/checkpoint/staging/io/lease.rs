//! 固定空文件的内核排他锁；原目录和锁 inode 每次复核，绝不删除或轮换锁文件。
use super::{Directory, FileType, Mode, OFlags, OwnedFd, Result, StagingError, acl, error, fs};

pub(in crate::checkpoint::staging) struct Lease {
    fd: OwnedFd,
    parent: (u64, u64),
}
pub(super) const NAME: &str = ".producer-lease";
impl Lease {
    pub(super) fn acquire(parent: &Directory) -> Result<Self> {
        Self::acquire_mode(parent, true)
    }
    pub(super) fn existing(parent: &Directory) -> Result<Self> {
        Self::acquire_mode(parent, false)
    }
    fn acquire_mode(parent: &Directory, create: bool) -> Result<Self> {
        parent.access.require_writer()?;
        let flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        let fd = if !create {
            fs::openat(&parent.fd, NAME, flags, Mode::empty()).map_err(error)?
        } else {
            match fs::openat(
                &parent.fd,
                NAME,
                flags | OFlags::CREATE | OFlags::EXCL,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(fd) => {
                    fs::fsync(&fd).map_err(error)?;
                    parent.sync()?;
                    fd
                }
                Err(rustix::io::Errno::EXIST) => {
                    fs::openat(&parent.fd, NAME, flags, Mode::empty()).map_err(error)?
                }
                Err(e) => return Err(error(e)),
            }
        };
        validate(&fd)?;
        fs::flock(&fd, fs::FlockOperation::NonBlockingLockExclusive).map_err(|e| {
            if e == rustix::io::Errno::AGAIN {
                StagingError::Busy
            } else {
                error(e)
            }
        })?;
        let lease = Self {
            fd,
            parent: parent.pin,
        };
        lease.check(parent)?;
        Ok(lease)
    }
    pub(in crate::checkpoint::staging) fn check(&self, parent: &Directory) -> Result<()> {
        parent.access.require_writer()?;
        let original = validate(&self.fd)?;
        let current = fs::statat(&parent.fd, NAME, fs::AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if parent.pin != self.parent
            || (current.st_dev, current.st_ino) != original
            || FileType::from_raw_mode(current.st_mode) != FileType::RegularFile
        {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
}
fn validate(fd: &OwnedFd) -> Result<(u64, u64)> {
    let st = fs::fstat(fd).map_err(error)?;
    if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile
        || st.st_uid != rustix::process::geteuid().as_raw()
        || st.st_mode & 0o7777 != 0o600
        || st.st_nlink != 1
        || st.st_size != 0
    {
        return Err(StagingError::UnsafeRoot);
    }
    acl::require_no_acl(fd, false)?;
    Ok((st.st_dev, st.st_ino))
}
