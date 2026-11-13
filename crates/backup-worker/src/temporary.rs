//! 只接受预先创建的 worker 私有临时根；调用前后核对同一个目录身份。
use crate::{ProcessError, Result};
use rustix::{
    fd::OwnedFd,
    fs::{self, Mode, OFlags},
};
use std::path::Path;

const ROOT: &str = "/var/lib/studiolo-backup-worker/tmp";
const FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
pub(crate) struct Temporary {
    fd: OwnedFd,
    pin: (u64, u64),
}
impl Temporary {
    pub fn open() -> Result<Self> {
        let uid = rustix::process::geteuid().as_raw();
        if uid == 0 {
            return Err(ProcessError::Configuration);
        }
        let mut fd =
            fs::open("/", FLAGS, Mode::empty()).map_err(|_| ProcessError::Configuration)?;
        for part in ["var", "lib", "studiolo-backup-worker", "tmp"] {
            fd = fs::openat(&fd, part, FLAGS, Mode::empty())
                .map_err(|_| ProcessError::Configuration)?;
            let stat = fs::fstat(&fd).map_err(|_| ProcessError::Configuration)?;
            if (stat.st_uid != 0 && stat.st_uid != uid) || stat.st_mode & 0o7022 != 0 {
                return Err(ProcessError::Configuration);
            }
        }
        let stat = fs::fstat(&fd).map_err(|_| ProcessError::Configuration)?;
        if stat.st_uid != uid || stat.st_mode & 0o7777 != 0o700 {
            return Err(ProcessError::Configuration);
        }
        Ok(Self {
            fd,
            pin: (stat.st_dev, stat.st_ino),
        })
    }
    pub fn path(&self) -> &Path {
        Path::new(ROOT)
    }
    pub fn check(&self) -> Result<()> {
        let original = fs::fstat(&self.fd).map_err(|_| ProcessError::Configuration)?;
        if (original.st_dev, original.st_ino) != self.pin || Self::open()?.pin != self.pin {
            return Err(ProcessError::Configuration);
        }
        Ok(())
    }
}
