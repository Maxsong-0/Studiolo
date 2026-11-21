//! 仅生产者以原 FD 回收本次目录；支持共享发布到一半，读取者仍没有删除能力。
//! 登记目录另走 `retire`：先原子改名成「删除中」的名字，再删，身份记录最后删。
use super::{Access, Directory, FileType, Mode, component, error, fs};
use crate::checkpoint::staging::{MAX_NODES, ReaderIdentity, Result, StagingError, budget::Budget};
use std::time::Instant;

impl Directory {
    pub fn discard_until(
        &self,
        parent: &Self,
        name: &str,
        reader: Option<ReaderIdentity>,
        deadline: Instant,
    ) -> Result<()> {
        self.discard_within(parent, name, reader, &mut Budget::until(deadline))
    }
    /// 同 `discard_until`；删除的每一步记在调用方的预算里。
    pub fn discard_within(
        &self,
        parent: &Self,
        name: &str,
        reader: Option<ReaderIdentity>,
        budget: &mut Budget,
    ) -> Result<()> {
        self.discard(parent, name, reader, None, budget)
    }
    /// 删登记目录：先把 `name` 原子改名成 `removing`（不覆盖已有的同名项），再删里面的登记文件，
    /// `last`（身份记录）留到最后，最后删目录。改名之后不管停在哪一步（期限到、出错、进程被杀），
    /// 下一轮都按 `removing` 认出它接着删；`name` 已经不在（上一次改过名）就直接接着删。
    pub fn retire(
        &self,
        parent: &Self,
        name: &str,
        removing: &str,
        last: &str,
        budget: &mut Budget,
    ) -> Result<()> {
        budget.check()?;
        self.may_discard(parent, None)?;
        if name != removing && parent.exists(name)? {
            self.cleanup_binding(parent, name, None)?;
            budget.spend()?;
            parent.publish(name, removing)?;
        }
        self.discard(parent, removing, None, Some(last), budget)
    }
    fn may_discard(&self, parent: &Self, reader: Option<ReaderIdentity>) -> Result<()> {
        parent.access.require_writer()?;
        match self.access {
            Access::Private => Ok(()),
            Access::SharingRoot(id) | Access::Shared(id)
                if Some(id) == reader && id.producer_uid == rustix::process::geteuid().as_raw() =>
            {
                Ok(())
            }
            _ => Err(StagingError::UnsafeRoot),
        }
    }
    fn discard(
        &self,
        parent: &Self,
        name: &str,
        reader: Option<ReaderIdentity>,
        last: Option<&str>,
        budget: &mut Budget,
    ) -> Result<()> {
        budget.check()?;
        self.may_discard(parent, reader)?;
        component(name)?;
        if !parent.exists(name)? {
            return Ok(());
        }
        self.cleanup_binding(parent, name, reader)?;
        let mut remaining = MAX_NODES;
        self.empty(reader, &mut remaining, 0, last, budget)?;
        self.cleanup_binding(parent, name, reader)?;
        budget.spend()?;
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
        last: Option<&str>,
        budget: &mut Budget,
    ) -> Result<()> {
        budget.check()?;
        if depth > 64 {
            return Err(StagingError::Limit);
        }
        self.cleanup_metadata(reader)?;
        // 原生产者收回组可读性后再删除；只操作已经核对身份的 FD。
        fs::fchmod(&self.fd, Mode::RWXU).map_err(error)?;
        let mut names = self.names(*remaining)?;
        *remaining -= names.len() as u64;
        // 登记目录的身份记录留到最后删：中途停下时，剩下的东西还认得出是谁的。
        if let Some(last) = last {
            names.sort_by_key(|name| name == last);
        }
        for name in &names {
            budget.check()?;
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
                child.empty(reader, remaining, depth + 1, None, budget)?;
                child.cleanup_binding(self, name, reader)?;
                budget.spend()?;
                fs::unlinkat(&self.fd, name, fs::AtFlags::REMOVEDIR).map_err(error)?;
            } else {
                // 不跟随符号链接；多链接文件也只删除本次目录项，不访问另一端。
                budget.spend()?;
                fs::unlinkat(&self.fd, name, fs::AtFlags::empty()).map_err(error)?;
            }
        }
        self.sync()
    }
}
