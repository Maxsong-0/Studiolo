//! 固定文件、只读原 FD、有界内容与再次路径核验；不创建缺失的配置。
use crate::{ProcessError, Result};
use rustix::fs::{self, Mode, OFlags};
use std::{
    fs::{File, Metadata},
    io::Read,
    os::unix::fs::MetadataExt,
    time::Instant,
};

const FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
pub(super) fn read(deadline: Instant) -> Result<Vec<u8>> {
    let (bytes, pin) = read_checked(open_config()?, 0, deadline)?;
    let reopened = open_config()?
        .metadata()
        .map_err(|_| ProcessError::Configuration)?;
    if !same(&pin, &reopened) {
        return Err(ProcessError::Configuration);
    }
    if Instant::now() >= deadline {
        return Err(ProcessError::Deadline);
    }
    Ok(bytes)
}
fn open_config() -> Result<File> {
    let mut directory =
        fs::open("/", FLAGS, Mode::empty()).map_err(|_| ProcessError::Configuration)?;
    for name in [None, Some("etc"), Some("studiolo")] {
        if let Some(name) = name {
            directory = fs::openat(&directory, name, FLAGS, Mode::empty())
                .map_err(|_| ProcessError::Configuration)?;
        }
        let stat = fs::fstat(&directory).map_err(|_| ProcessError::Configuration)?;
        if stat.st_uid != 0 || stat.st_mode & 0o7022 != 0 {
            return Err(ProcessError::Configuration);
        }
    }
    let fd = fs::openat(
        &directory,
        "backup-worker.json",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| ProcessError::Configuration)?;
    Ok(File::from(fd))
}
fn read_checked(mut file: File, owner: u32, deadline: Instant) -> Result<(Vec<u8>, Metadata)> {
    if Instant::now() >= deadline {
        return Err(ProcessError::Deadline);
    }
    let pin = file.metadata().map_err(|_| ProcessError::Configuration)?;
    if !pin.is_file()
        || pin.uid() != owner
        || pin.nlink() != 1
        || !matches!(pin.mode() & 0o7777, 0o644 | 0o444)
        || pin.len() == 0
        || pin.len() > 16 * 1024
    {
        return Err(ProcessError::Configuration);
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ProcessError::Configuration)?;
    let after = file.metadata().map_err(|_| ProcessError::Configuration)?;
    if bytes.len() as u64 != pin.len() || !same(&pin, &after) {
        return Err(ProcessError::Configuration);
    }
    if Instant::now() >= deadline {
        return Err(ProcessError::Deadline);
    }
    Ok((bytes, pin))
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
#[cfg(test)]
#[path = "../../tests/support/config_input.rs"]
mod tests;
