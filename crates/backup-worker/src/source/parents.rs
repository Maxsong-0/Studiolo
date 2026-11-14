//! 逐级 nofollow 重开源路径；只允许 root/生产者写祖先，并固定每一级原 inode。
use crate::{ProcessError, Result};
use rustix::{
    fd::OwnedFd,
    fs::{self, Mode, OFlags, Stat},
};
use std::{
    ffi::OsStr,
    path::{Component, Path},
    time::Instant,
};

const FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Parents(Vec<Pin>);
#[derive(Debug, PartialEq, Eq)]
struct Pin {
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
    mode: u32,
}
impl Parents {
    pub fn read(root: &Path, producer: u32, group: u32, deadline: Instant) -> Result<Self> {
        if !root.is_absolute() {
            return Err(ProcessError::Source);
        }
        let parts = root
            .components()
            .filter_map(|part| match part {
                Component::RootDir => None,
                Component::Normal(name) => Some(Ok(name)),
                _ => Some(Err(ProcessError::Source)),
            })
            .collect::<Result<Vec<_>>>()?;
        let fd = fs::open("/", FLAGS, Mode::empty()).map_err(|_| ProcessError::Source)?;
        Self::descend(fd, &parts, producer, group, deadline)
    }
    fn descend(
        mut fd: OwnedFd,
        parts: &[&OsStr],
        producer: u32,
        group: u32,
        deadline: Instant,
    ) -> Result<Self> {
        if parts.len() < 2 || parts.len() > 64 {
            return Err(ProcessError::Source);
        }
        let mut pins = vec![pin(&fd, producer)?];
        for (index, name) in parts.iter().enumerate() {
            if Instant::now() >= deadline {
                return Err(ProcessError::Deadline);
            }
            fd = fs::openat(&fd, *name, FLAGS, Mode::empty()).map_err(|_| ProcessError::Source)?;
            let item = pin(&fd, producer)?;
            if index + 2 >= parts.len() {
                let mode = if index + 1 == parts.len() {
                    0o550
                } else {
                    0o750
                };
                if item.uid != producer || item.gid != group || item.mode & 0o7777 != mode {
                    return Err(ProcessError::Source);
                }
            }
            pins.push(item);
        }
        Ok(Self(pins))
    }
}
fn pin(fd: &OwnedFd, producer: u32) -> Result<Pin> {
    let stat = fs::fstat(fd).map_err(|_| ProcessError::Source)?;
    validate(&stat, producer)?;
    for name in ["system.posix_acl_access", "system.posix_acl_default"] {
        let mut byte = [0_u8; 1];
        match fs::fgetxattr(fd, name, &mut byte[..]) {
            Err(rustix::io::Errno::NODATA | rustix::io::Errno::OPNOTSUPP) => {}
            _ => return Err(ProcessError::Source),
        }
    }
    Ok(Pin {
        device: stat.st_dev,
        inode: stat.st_ino,
        uid: stat.st_uid,
        gid: stat.st_gid,
        mode: stat.st_mode,
    })
}
fn validate(stat: &Stat, producer: u32) -> Result<()> {
    if (stat.st_uid != 0 && stat.st_uid != producer) || stat.st_mode & 0o7022 != 0 {
        return Err(ProcessError::Source);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/support/source_parents.rs"]
mod tests;
