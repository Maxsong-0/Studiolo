//! 本目录的路径能力仅在这里打开；逐层 nofollow，后续操作只相对已持有 FD。
mod access;
mod acl;
mod cleanup;
mod files;
mod lease;
mod share;
use super::{Result, StagingError};
use access::Access;
pub(super) use files::Regular;
pub(super) use lease::Lease;
use rustix::{
    fd::OwnedFd,
    fs::{self, FileType, Mode, OFlags},
};
use std::path::{Component, Path};

pub(super) struct Directory {
    fd: OwnedFd,
    pin: (u64, u64),
    access: Access,
}
const FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);
impl Directory {
    pub fn acquire_lease(&self) -> Result<Lease> {
        Lease::acquire(self)
    }
    pub fn open(path: &Path, private: bool) -> Result<Self> {
        Self::open_with(
            path,
            if private {
                Access::Private
            } else {
                Access::Source
            },
        )
    }
    pub fn publisher(path: &Path, identity: super::ReaderIdentity) -> Result<Self> {
        Self::open_with(path, Access::Publisher(identity))
    }
    pub fn reader_parent(path: &Path, identity: super::ReaderIdentity) -> Result<Self> {
        Self::open_with(path, Access::SharedParent(identity))
    }
    fn open_with(path: &Path, access: Access) -> Result<Self> {
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(StagingError::UnsafeRoot);
        }
        let mut fd = fs::open("/", FLAGS, Mode::empty()).map_err(error)?;
        for component in path.components() {
            if let Component::Normal(name) = component {
                fd = fs::openat(&fd, name, FLAGS, Mode::empty()).map_err(error)?;
            }
        }
        Self::from_fd(fd, access)
    }
    fn from_fd(fd: OwnedFd, access: Access) -> Result<Self> {
        let st = fs::fstat(&fd).map_err(error)?;
        if FileType::from_raw_mode(st.st_mode) != FileType::Directory
            || !access.directory(st.st_uid, st.st_gid, st.st_mode)
        {
            return Err(StagingError::UnsafeRoot);
        }
        if access != Access::Source {
            acl::require_no_acl(&fd, true)?;
        }
        Ok(Self {
            fd,
            pin: (st.st_dev, st.st_ino),
            access,
        })
    }
    pub fn child(&self, name: &str) -> Result<Self> {
        self.child_with(name, self.access.child())
    }
    fn child_with(&self, name: &str, access: Access) -> Result<Self> {
        component(name)?;
        let child = Self::from_fd(
            fs::openat(&self.fd, name, FLAGS, Mode::empty()).map_err(error)?,
            access,
        )?;
        if child.pin.0 != self.pin.0 {
            return Err(StagingError::UnsafeRoot);
        }
        Ok(child)
    }
    pub fn create_dir(&self, name: &str) -> Result<Self> {
        self.access.require_writer()?;
        component(name)?;
        self.capacity(4096)?;
        fs::mkdirat(&self.fd, name, Mode::RWXU).map_err(error)?;
        self.sync()?;
        self.child(name)
    }
    pub fn check_path(&self, path: &Path) -> Result<()> {
        if Self::open_with(path, self.access)?.pin != self.pin {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
    pub fn check_child(&self, parent: &Self, name: &str) -> Result<()> {
        if parent.child(name)?.pin != self.pin {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
    pub fn names(&self, remaining: u64) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let entries = fs::Dir::read_from(&self.fd).map_err(error)?;
        for entry in entries {
            let entry = entry.map_err(error)?;
            let name = entry
                .file_name()
                .to_str()
                .map_err(|_| StagingError::UnsafeRoot)?;
            if matches!(name, "." | "..") {
                continue;
            }
            component(name)?;
            if names.len() as u64 >= remaining {
                return Err(StagingError::Limit);
            }
            names.push(name.to_owned());
        }
        names.sort();
        Ok(names)
    }
    pub fn is_directory(&self, name: &str) -> Result<bool> {
        component(name)?;
        let st = fs::statat(&self.fd, name, fs::AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        match FileType::from_raw_mode(st.st_mode) {
            FileType::Directory => Ok(true),
            FileType::RegularFile => Ok(false),
            _ => Err(StagingError::UnsafeRoot),
        }
    }
    pub fn exists(&self, name: &str) -> Result<bool> {
        component(name)?;
        match fs::statat(&self.fd, name, fs::AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => Ok(true),
            Err(rustix::io::Errno::NOENT) => Ok(false),
            Err(e) => Err(error(e)),
        }
    }
    pub fn sync(&self) -> Result<()> {
        fs::fsync(&self.fd).map_err(error)
    }
    pub fn readonly(&self) -> Result<()> {
        self.access.require_writer()?;
        fs::fchmod(&self.fd, Mode::RUSR | Mode::XUSR).map_err(error)?;
        self.sync()
    }
    pub fn require_readonly(&self) -> Result<()> {
        acl::require_no_acl(&self.fd, true)?;
        let st = fs::fstat(&self.fd).map_err(error)?;
        if !self.access.directory(st.st_uid, st.st_gid, st.st_mode)
            || st.st_mode & 0o7777 != self.access.readonly_directory()
        {
            return Err(StagingError::UnsafeRoot);
        }
        Ok(())
    }
    pub fn capacity(&self, incoming: u64) -> Result<()> {
        let st = fs::fstatvfs(&self.fd).map_err(error)?;
        let needed = (2 * 1024 * 1024 * 1024_u64)
            .checked_add(incoming)
            .ok_or(StagingError::Limit)?;
        if st
            .f_bavail
            .checked_mul(st.f_frsize)
            .is_none_or(|v| v < needed)
        {
            return Err(StagingError::Limit);
        }
        Ok(())
    }
    pub fn publish(&self, pending: &str, final_name: &str) -> Result<()> {
        self.access.require_writer()?;
        component(pending)?;
        component(final_name)?;
        // Linux 的 NOREPLACE 是原子性要求，不能用「先 exists 再覆盖式 rename」。
        #[cfg(target_os = "linux")]
        {
            fs::renameat_with(
                &self.fd,
                pending,
                &self.fd,
                final_name,
                fs::RenameFlags::NOREPLACE,
            )
            .map_err(error)?;
            self.sync()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(StagingError::Unavailable)
        }
    }
}
pub(super) fn disjoint(a: &Path, left: &Directory, b: &Path, right: &Directory) -> Result<()> {
    if a.starts_with(b) || b.starts_with(a) || ancestor(a, right.pin)? || ancestor(b, left.pin)? {
        return Err(StagingError::UnsafeRoot);
    }
    Ok(())
}
fn ancestor(path: &Path, wanted: (u64, u64)) -> Result<bool> {
    let mut fd = fs::open("/", FLAGS, Mode::empty()).map_err(error)?;
    for component in path.components() {
        if let Component::Normal(name) = component {
            fd = fs::openat(&fd, name, FLAGS, Mode::empty()).map_err(error)?;
        }
        let st = fs::fstat(&fd).map_err(error)?;
        if (st.st_dev, st.st_ino) == wanted {
            return Ok(true);
        }
    }
    Ok(false)
}
fn component(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 255
        || matches!(name, "." | "..")
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        return Err(StagingError::UnsafeRoot);
    }
    Ok(())
}
fn error(e: rustix::io::Errno) -> StagingError {
    match e {
        rustix::io::Errno::EXIST => StagingError::AlreadyExists,
        rustix::io::Errno::NOSPC | rustix::io::Errno::DQUOT => StagingError::Limit,
        _ => StagingError::Unavailable,
    }
}
