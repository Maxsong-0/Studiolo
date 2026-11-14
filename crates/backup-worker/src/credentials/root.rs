//! systemd 固定凭据根：拒绝可写、可换出和普通目录降级，保留原路径每一级身份。
use crate::{ProcessError, Result};
use rustix::{
    fd::{AsFd, OwnedFd},
    fs::{self, Mode, OFlags},
};
use std::time::Instant;

pub(super) const ROOT: &str = "/run/credentials/studiolo-backup-worker.service";
const FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
pub(super) struct Root {
    fd: OwnedFd,
    pins: Vec<(u64, u64, u32, u32, u32)>,
    uid: u32,
}
impl Root {
    pub fn open_credentials(uid: u32, deadline: Instant) -> Result<Self> {
        if uid == 0 || uid != rustix::process::geteuid().as_raw() {
            return Err(ProcessError::Configuration);
        }
        let mut fd =
            fs::open("/", FLAGS, Mode::empty()).map_err(|_| ProcessError::Configuration)?;
        let mut pins = Vec::new();
        for name in [
            None,
            Some("run"),
            Some("credentials"),
            Some("studiolo-backup-worker.service"),
        ] {
            check_deadline(deadline)?;
            if let Some(name) = name {
                fd = fs::openat(&fd, name, FLAGS, Mode::empty())
                    .map_err(|_| ProcessError::Configuration)?;
            }
            let stat = fs::fstat(&fd).map_err(|_| ProcessError::Configuration)?;
            no_acl(&fd)?;
            if name == Some("studiolo-backup-worker.service") {
                if stat.st_uid != uid || stat.st_mode & 0o7777 != 0o500 {
                    return Err(ProcessError::Configuration);
                }
                readonly_memory(&fd)?;
            } else if stat.st_uid != 0 || stat.st_mode & 0o7022 != 0 {
                return Err(ProcessError::Configuration);
            }
            pins.push((
                stat.st_dev,
                stat.st_ino,
                stat.st_uid,
                stat.st_gid,
                stat.st_mode,
            ));
        }
        check_deadline(deadline)?;
        Ok(Self { fd, pins, uid })
    }
    pub fn fd(&self) -> &OwnedFd {
        &self.fd
    }
    pub fn uid(&self) -> u32 {
        self.uid
    }
    pub fn check(&self, deadline: Instant) -> Result<()> {
        let reopened = Self::open_credentials(self.uid, deadline)?;
        let original = fs::fstat(&self.fd).map_err(|_| ProcessError::Configuration)?;
        readonly_memory(&self.fd)?;
        no_acl(&self.fd)?;
        if reopened.pins != self.pins
            || self.pins.last()
                != Some(&(
                    original.st_dev,
                    original.st_ino,
                    original.st_uid,
                    original.st_gid,
                    original.st_mode,
                ))
        {
            return Err(ProcessError::Configuration);
        }
        check_deadline(deadline)
    }
}
pub(super) fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(ProcessError::Deadline)
    } else {
        Ok(())
    }
}
pub(super) fn no_acl(fd: &impl AsFd) -> Result<()> {
    for name in ["system.posix_acl_access", "system.posix_acl_default"] {
        let mut byte = [0_u8; 1];
        match fs::fgetxattr(fd, name, &mut byte[..]) {
            Err(rustix::io::Errno::NODATA | rustix::io::Errno::OPNOTSUPP) => {}
            _ => return Err(ProcessError::Configuration),
        }
    }
    Ok(())
}
pub(super) fn readonly_memory(fd: &impl AsFd) -> Result<()> {
    let stat = fs::fstatfs(fd).map_err(|_| ProcessError::Configuration)?;
    let mount = fs::fstatvfs(fd).map_err(|_| ProcessError::Configuration)?;
    if stat.f_type as u64 != 0x8584_58f6 || !mount.f_flag.contains(fs::StatVfsMountFlags::RDONLY) {
        return Err(ProcessError::Configuration);
    }
    Ok(())
}
