//! 固定安装路径逐级 nofollow；执行原 FD，拒绝版本文件被换掉或原 inode 被改写。
use crate::{
    ProcessError, Result,
    digest::{hex, is_sha256_hex},
};
use rustix::fs::{self, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata},
    io::{Read, Seek},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Component, Path},
    time::Instant,
};

const PACKAGE_PATH: &str = "/usr/lib/studiolo-backup-worker/restic-0.19.1";
const MAX_BINARY: u64 = 128 * 1024 * 1024;
const FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// 摘要已经匹配受保护部署清单的原可执行文件。没有自由指定命令路径的生产入口。
pub struct ResticPackage {
    file: File,
    pin: Metadata,
}
impl ResticPackage {
    /// 固定 root 所有的安装位置；摘要必须来自已批准部署清单，不能读旁边自报的摘要文件。
    pub fn open(expected_sha256: &str, deadline: Instant) -> Result<Self> {
        let path = Path::new(PACKAGE_PATH);
        let mut directory =
            fs::open("/", FLAGS, Mode::empty()).map_err(|_| ProcessError::Package)?;
        check_directory(&directory)?;
        for part in path.parent().ok_or(ProcessError::Package)?.components() {
            if let Component::Normal(name) = part {
                directory = fs::openat(&directory, name, FLAGS, Mode::empty())
                    .map_err(|_| ProcessError::Package)?;
                check_directory(&directory)?;
            }
        }
        let fd = fs::openat(
            &directory,
            "restic-0.19.1",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| ProcessError::Package)?;
        Self::from_file(File::from(fd), expected_sha256, 0, deadline)
    }
    fn from_file(mut file: File, hash: &str, owner: u32, deadline: Instant) -> Result<Self> {
        let pin = file.metadata().map_err(|_| ProcessError::Package)?;
        if !pin.is_file()
            || pin.nlink() != 1
            || pin.uid() != owner
            || !matches!(pin.mode() & 0o7777, 0o755 | 0o555 | 0o500)
            || pin.len() == 0
            || pin.len() > MAX_BINARY
            || !is_sha256_hex(hash)
        {
            return Err(ProcessError::Package);
        }
        let mut digest = Sha256::new();
        let mut bytes = 0_u64;
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            if Instant::now() >= deadline {
                return Err(ProcessError::Deadline);
            }
            let read = file.read(&mut chunk).map_err(|_| ProcessError::Package)?;
            if read == 0 {
                break;
            }
            if bytes == 0 && !chunk[..read].starts_with(b"\x7fELF") {
                return Err(ProcessError::Package);
            }
            bytes += read as u64;
            if bytes > pin.len() {
                return Err(ProcessError::Package);
            }
            digest.update(&chunk[..read]);
        }
        let actual = hex(&digest.finalize());
        if bytes != pin.len() || actual != hash {
            return Err(ProcessError::Package);
        }
        file.rewind().map_err(|_| ProcessError::Package)?;
        let this = Self { file, pin };
        this.check()?;
        Ok(this)
    }
    pub(crate) fn executable(&self) -> Result<String> {
        self.check()?;
        Ok(format!("/proc/self/fd/{}", self.file.as_raw_fd()))
    }
    pub(crate) fn check(&self) -> Result<()> {
        let stat = self.file.metadata().map_err(|_| ProcessError::Package)?;
        if !same(&self.pin, &stat) {
            return Err(ProcessError::Package);
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn fixture(path: &Path, hash: &str, deadline: Instant) -> Result<Self> {
        let file = File::open(path).map_err(|_| ProcessError::Package)?;
        Self::from_file(file, hash, rustix::process::geteuid().as_raw(), deadline)
    }
}
fn check_directory(fd: &impl rustix::fd::AsFd) -> Result<()> {
    let stat = fs::fstat(fd).map_err(|_| ProcessError::Package)?;
    if stat.st_uid != 0 || stat.st_mode & 0o7022 != 0 {
        return Err(ProcessError::Package);
    }
    Ok(())
}
fn same(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.uid() == b.uid()
        && a.gid() == b.gid()
        && a.mode() == b.mode()
        && a.nlink() == b.nlink()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}
