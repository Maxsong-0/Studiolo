use rustix::{
    fd::{AsRawFd, OwnedFd},
    fs::{self, FileType, Mode, OFlags, Stat},
};
use std::{
    ffi::OsStr,
    path::{Component, Path},
    time::Instant,
};
use studiolo_audit_ledger::AuditError;
const FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
#[derive(PartialEq, Eq)]
struct Pin {
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
    mode: u32,
}
pub(super) struct Proof {
    pins: Vec<Pin>,
    _fds: Vec<OwnedFd>,
}
impl Proof {
    pub fn open(
        path: &Path,
        producer: u32,
        group: u32,
        deadline: Instant,
    ) -> Result<Self, AuditError> {
        if !path.is_absolute() || matches!(producer, 0 | u32::MAX) || matches!(group, 0 | u32::MAX)
        {
            return Err(AuditError::UnsafeStorage);
        }
        let parts = path
            .components()
            .filter_map(|p| match p {
                Component::RootDir => None,
                Component::Normal(name) => Some(Ok(name)),
                _ => Some(Err(AuditError::UnsafeStorage)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let root = fs::open("/", FLAGS, Mode::empty()).map_err(|_| AuditError::UnsafeStorage)?;
        Self::descend(root, &parts, producer, group, deadline)
    }
    fn descend(
        root: OwnedFd,
        parts: &[&OsStr],
        producer: u32,
        group: u32,
        deadline: Instant,
    ) -> Result<Self, AuditError> {
        if parts.len() < 2 || parts.len() > 64 {
            return Err(AuditError::UnsafeStorage);
        }
        let mut proof = Self {
            pins: Vec::new(),
            _fds: Vec::new(),
        };
        proof.pins.push(directory(&root, producer)?);
        proof._fds.push(root);
        for (index, name) in parts.iter().enumerate() {
            if Instant::now() >= deadline {
                return Err(AuditError::Unavailable);
            }
            let parent = proof._fds.last().ok_or(AuditError::UnsafeStorage)?;
            let final_component = index + 1 == parts.len();
            let fd = fs::openat(
                parent,
                *name,
                if final_component {
                    OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC
                } else {
                    FLAGS
                },
                Mode::empty(),
            )
            .map_err(|_| AuditError::UnsafeStorage)?;
            let item = if final_component {
                let parent_pin = proof.pins.last().ok_or(AuditError::UnsafeStorage)?;
                if parent_pin.uid != producer
                    || parent_pin.gid != group
                    || parent_pin.mode & 0o7777 != 0o750
                {
                    return Err(AuditError::UnsafeStorage);
                }
                socket(&fd, producer, group)?
            } else {
                directory(&fd, producer)?
            };
            proof.pins.push(item);
            proof._fds.push(fd);
        }
        if Instant::now() >= deadline {
            return Err(AuditError::Unavailable);
        }
        Ok(proof)
    }
    pub fn same(&self, other: &Self) -> Result<(), AuditError> {
        if self.pins == other.pins {
            Ok(())
        } else {
            Err(AuditError::UnsafeStorage)
        }
    }
}
fn directory(fd: &OwnedFd, producer: u32) -> Result<Pin, AuditError> {
    let stat = fs::fstat(fd).map_err(|_| AuditError::UnsafeStorage)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || (stat.st_uid != 0 && stat.st_uid != producer)
        || stat.st_mode & 0o7022 != 0
    {
        return Err(AuditError::UnsafeStorage);
    }
    for name in ["system.posix_acl_access", "system.posix_acl_default"] {
        let mut byte = [0_u8; 1];
        match fs::fgetxattr(fd, name, &mut byte[..]) {
            Err(rustix::io::Errno::NODATA | rustix::io::Errno::OPNOTSUPP) => {}
            _ => return Err(AuditError::UnsafeStorage),
        }
    }
    Ok(pin(&stat))
}
fn socket(fd: &OwnedFd, producer: u32, group: u32) -> Result<Pin, AuditError> {
    let stat = fs::fstat(fd).map_err(|_| AuditError::UnsafeStorage)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Socket
        || stat.st_uid != producer
        || stat.st_gid != group
        || stat.st_mode & 0o7777 != 0o660
        || stat.st_nlink != 1
    {
        return Err(AuditError::UnsafeStorage);
    }
    // O_PATH 不能 fgetxattr；proc magic link 只指向本次保留的原 FD，不重新解析配置路径。
    let pinned = format!("/proc/self/fd/{}", fd.as_raw_fd());
    let mut byte = [0_u8; 1];
    match fs::getxattr(&pinned, "system.posix_acl_access", &mut byte[..]) {
        Err(rustix::io::Errno::NODATA | rustix::io::Errno::OPNOTSUPP) => {}
        _ => return Err(AuditError::UnsafeStorage),
    }
    Ok(pin(&stat))
}
fn pin(stat: &Stat) -> Pin {
    Pin {
        device: stat.st_dev,
        inode: stat.st_ino,
        uid: stat.st_uid,
        gid: stat.st_gid,
        mode: stat.st_mode,
    }
}
#[cfg(test)]
#[path = "../../../tests/support/audit_endpoint_proof.rs"]
mod tests;
