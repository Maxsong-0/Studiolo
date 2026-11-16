//! 仅生产者以原 FD 回收本次目录；支持共享发布到一半，读取者仍没有删除能力。
use super::{Access, Directory, FileType, Mode, component, error, fs};
use crate::checkpoint::staging::{MAX_NODES, ReaderIdentity, Result, StagingError};
use std::time::Instant;

impl Directory {
    pub fn discard_until(
        &self,
        parent: &Self,
        name: &str,
        reader: Option<ReaderIdentity>,
        deadline: Instant,
    ) -> Result<()> {
        crate::checkpoint::staging::check_deadline(deadline)?;
        parent.access.require_writer()?;
        match self.access {
            Access::Private => {}
            Access::SharingRoot(id) | Access::Shared(id)
                if Some(id) == reader && id.producer_uid == rustix::process::geteuid().as_raw() => {
            }
            _ => return Err(StagingError::UnsafeRoot),
        }
        component(name)?;
        if !parent.exists(name)? {
            return Ok(());
        }
        self.cleanup_binding(parent, name, reader)?;
        let mut remaining = MAX_NODES;
        self.empty(reader, &mut remaining, 0, deadline)?;
        self.cleanup_binding(parent, name, reader)?;
        fs::unlinkat(&parent.fd, name, fs::AtFlags::REMOVEDIR).map_err(error)?;
        parent.sync()
    }
    pub fn reclaim_child(
        &self,
        name: &str,
        expected: (u64, u64),
        reader: Option<ReaderIdentity>,
    ) -> Result<Self> {
        self.access.require_writer()?;
        component(name)?;
        let fd = fs::openat(&self.fd, name, super::FLAGS, Mode::empty()).map_err(error)?;
        let st = fs::fstat(&fd).map_err(error)?;
        if (st.st_dev, st.st_ino) != expected {
            return Err(StagingError::Changed);
        }
        let access = match (st.st_mode & 0o7777, reader) {
            (0o550, Some(id)) => Access::Shared(id),
            _ => Access::Private,
        };
        let child = Self {
            fd,
            pin: expected,
            access,
        };
        child.cleanup_binding(self, name, reader)?;
        Ok(child)
    }
    fn cleanup_binding(
        &self,
        parent: &Self,
        name: &str,
        reader: Option<ReaderIdentity>,
    ) -> Result<()> {
        self.cleanup_metadata(reader)?;
        let st = fs::statat(&parent.fd, name, fs::AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if FileType::from_raw_mode(st.st_mode) != FileType::Directory
            || (st.st_dev, st.st_ino) != self.pin
            || st.st_dev != parent.pin.0
        {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
    fn cleanup_metadata(&self, reader: Option<ReaderIdentity>) -> Result<()> {
        let st = fs::fstat(&self.fd).map_err(error)?;
        let mode = st.st_mode & 0o7777;
        let private = matches!(mode, 0o700 | 0o500);
        let shared = mode == 0o550
            && reader.is_some_and(|id| id.producer_uid == st.st_uid && id.reader_gid == st.st_gid);
        if st.st_uid != rustix::process::geteuid().as_raw()
            || (st.st_dev, st.st_ino) != self.pin
            || FileType::from_raw_mode(st.st_mode) != FileType::Directory
            || (!private && !shared)
        {
            return Err(StagingError::UnsafeRoot);
        }
        super::acl::require_no_acl(&self.fd, true)
    }
    fn empty(
        &self,
        reader: Option<ReaderIdentity>,
        remaining: &mut u64,
        depth: usize,
        deadline: Instant,
    ) -> Result<()> {
        crate::checkpoint::staging::check_deadline(deadline)?;
        if depth > 64 {
            return Err(StagingError::Limit);
        }
        self.cleanup_metadata(reader)?;
        // 原生产者收回组可读性后再删除；只操作已经核对身份的 FD。
        fs::fchmod(&self.fd, Mode::RWXU).map_err(error)?;
        let names = self.names(*remaining)?;
        *remaining -= names.len() as u64;
        for name in &names {
            crate::checkpoint::staging::check_deadline(deadline)?;
            let stat = fs::statat(&self.fd, name, fs::AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
            if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
                let fd = fs::openat(&self.fd, name, super::FLAGS, Mode::empty()).map_err(error)?;
                let actual = fs::fstat(&fd).map_err(error)?;
                let child = Self {
                    fd,
                    pin: (actual.st_dev, actual.st_ino),
                    access: Access::Private,
                };
                child.cleanup_binding(self, name, reader)?;
                if child.pin != (stat.st_dev, stat.st_ino) {
                    return Err(StagingError::Changed);
                }
                child.empty(reader, remaining, depth + 1, deadline)?;
                child.cleanup_binding(self, name, reader)?;
                fs::unlinkat(&self.fd, name, fs::AtFlags::REMOVEDIR).map_err(error)?;
            } else {
                // 不跟随符号链接；多链接文件也只删除本次目录项，不访问另一端。
                fs::unlinkat(&self.fd, name, fs::AtFlags::empty()).map_err(error)?;
            }
        }
        self.sync()
    }
}
