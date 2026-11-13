//! 文件内容只经固定缓冲读取，创建用 EXCL；前后核同一 FD 与重新打开的路径。
use super::{Directory, FileType, Mode, OFlags, access::Access, component, error, fs};
use crate::checkpoint::staging::{Result, StagingError};
use std::{
    fs::{File, Metadata},
    io::{Read, Write},
    os::unix::fs::MetadataExt,
};

pub(in crate::checkpoint::staging) struct Regular {
    file: File,
    pin: Metadata,
    access: Access,
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
        Ok(Self { file, pin, access })
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
            .map_err(|_| StagingError::Unavailable)
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
