//! 文件内容只经固定缓冲读取，创建用 EXCL；前后核同一 FD 与重新打开的路径。
use super::{Directory, FileType, Mode, OFlags, access::Access, component, error, fs};
use crate::checkpoint::staging::{Result, StagingError};
use std::{
    fs::{File, Metadata},
    io::{Read, Write},
    os::unix::fs::MetadataExt,
};
use studiolo_core::Flush;

pub(in crate::checkpoint::staging) struct Regular {
    file: File,
    pin: Metadata,
    access: Access,
    flush: Flush,
}
impl Regular {
    pub fn open(parent: &Directory, name: &str) -> Result<Self> {
        component(name)?;
        let fd = fs::openat(
            &parent.fd,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(error)?;
        Self::new(File::from(fd), parent.pin.0, parent.access)
    }
    pub fn create(parent: &Directory, name: &str, size: u64) -> Result<Self> {
        parent.access.require_writer()?;
        component(name)?;
        parent.capacity(size)?;
        let fd = fs::openat(
            &parent.fd,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(error)?;
        Self::new(File::from(fd), parent.pin.0, parent.access)
    }
    fn new(file: File, device: u64, access: Access) -> Result<Self> {
        super::acl::require_no_acl(&file, false)?;
        let pin = file.metadata().map_err(|_| StagingError::Unavailable)?;
        if FileType::from_raw_mode(pin.mode()) != FileType::RegularFile
            || pin.nlink() != 1
            || pin.dev() != device
            || !access.file(pin.uid(), pin.gid(), pin.mode())
        {
            return Err(StagingError::UnsafeRoot);
        }
        Ok(Self {
            file,
            pin,
            access,
            flush: Flush::default(),
        })
    }
    pub fn len(&self) -> u64 {
        self.pin.len()
    }
    pub fn require_readonly(&self) -> Result<()> {
        if self.pin.mode() & 0o7777 != self.access.readonly_file() {
            return Err(StagingError::UnsafeRoot);
        }
        Ok(())
    }
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.file
            .read(buffer)
            .map_err(|_| StagingError::Unavailable)
    }
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.access.require_writer()?;
        self.file
            .write_all(bytes)
            .map_err(|_| StagingError::Unavailable)?;
        // 写大文件时没落盘的量封顶（BUG-054）：账本有几百 MB，攒到 `finish` 才落的话，正在落盘的量就有整个账本那么大。
        let file = &self.file;
        self.flush.written(bytes.len() as u64, || flush(file))
    }
    pub fn check(&self, parent: &Directory, name: &str) -> Result<()> {
        let after = self
            .file
            .metadata()
            .map_err(|_| StagingError::Unavailable)?;
        let reopened = Self::open(parent, name)?;
        if !same(&self.pin, &after) || !same(&after, &reopened.pin) {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
    pub fn finish(self) -> Result<()> {
        self.access.require_writer()?;
        self.file
            .sync_all()
            .map_err(|_| StagingError::Unavailable)?;
        fs::fchmod(&self.file, Mode::RUSR).map_err(error)?;
        self.file.sync_all().map_err(|_| StagingError::Unavailable)
    }
    /// 恢复到新目录的文件要保持属主可写（各库接下来还要在上面写）：只落盘，不改成只读。
    pub fn finish_writable(self) -> Result<()> {
        self.access.require_writer()?;
        self.file.sync_all().map_err(|_| StagingError::Unavailable)
    }
    pub(super) fn grant_reader(self, reader_gid: u32) -> Result<()> {
        self.access.require_writer()?;
        self.require_readonly()?;
        fs::fchown(
            &self.file,
            None,
            Some(rustix::process::Gid::from_raw(reader_gid)),
        )
        .map_err(error)?;
        fs::fchmod(&self.file, Mode::RUSR | Mode::RGRP).map_err(error)?;
        self.file.sync_all().map_err(|_| StagingError::Unavailable)
    }
}
/// 写到一半先落一次盘。
fn flush(file: &File) -> Result<()> {
    file.sync_data().map_err(|_| StagingError::Unavailable)?;
    #[cfg(test)]
    {
        let length = file
            .metadata()
            .map_err(|_| StagingError::Unavailable)?
            .len();
        FLUSHED.with_borrow_mut(|lengths| lengths.push(length));
    }
    Ok(())
}
fn same(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mode() == b.mode()
        && a.nlink() == b.nlink()
        && a.uid() == b.uid()
        && a.gid() == b.gid()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}

#[cfg(test)]
thread_local! {
    /// 用例用：本线程写到一半每次落盘时，文件有多长。
    static FLUSHED: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
#[path = "../../../../tests/private/staging_flush.rs"]
mod tests;
