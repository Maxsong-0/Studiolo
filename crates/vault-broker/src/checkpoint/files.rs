//! 流式读同一 FD，前后重开路径核对 inode/时间/长度；任何替换或增长都拒绝整份封存。
use crate::BrokerError;
use cap_std::fs::Dir;
use rustix::fs::{Mode, OFlags, openat};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata},
    io::{Read, Write},
    os::unix::fs::MetadataExt as _,
    time::Instant,
};

fn open(parent: &Dir, name: &str) -> Result<File, BrokerError> {
    let fd = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| BrokerError::io("重开封存源文件", e.into()))?;
    Ok(File::from(fd))
}
fn metadata(file: &File, device: u64) -> Result<Metadata, BrokerError> {
    let value = file
        .metadata()
        .map_err(|e| BrokerError::io("核对封存文件身份", e))?;
    if !value.is_file()
        || value.nlink() != 1
        || value.dev() != device
        || value.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(BrokerError::IncompleteCapture);
    }
    Ok(value)
}
fn same(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mode() == right.mode()
        && left.nlink() == right.nlink()
        && left.uid() == right.uid()
        && left.gid() == right.gid()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}
pub(super) fn stream(
    parent: &Dir,
    name: &str,
    target: Option<&Dir>,
    device: u64,
    remaining: u64,
    deadline: Instant,
    buffer: &mut [u8],
) -> Result<([u8; 32], u64), BrokerError> {
    super::check_deadline(deadline)?;
    let mut source = open(parent, name)?;
    let before = metadata(&source, device)?;
    if before.len() > remaining {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let mut output = target
        .map(|dir| super::copy::file(dir, name, before.len()))
        .transpose()?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    loop {
        super::check_deadline(deadline)?;
        let read = source
            .read(buffer)
            .map_err(|e| BrokerError::io("流式读取封存源", e))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if total > before.len() {
            return Err(BrokerError::CaptureChanged);
        }
        hash.update(&buffer[..read]);
        if let Some(output) = &mut output {
            output
                .write_all(&buffer[..read])
                .map_err(|e| BrokerError::io("流式写入封存副本", e))?;
        }
    }
    let after = metadata(&source, device)?;
    let current = metadata(&open(parent, name)?, device)?;
    if total != before.len() || !same(&before, &after) || !same(&after, &current) {
        return Err(BrokerError::CaptureChanged);
    }
    if let Some(output) = output {
        output
            .sync_all()
            .map_err(|e| BrokerError::io("同步封存文件", e))?;
    }
    super::check_deadline(deadline)?;
    Ok((hash.finalize().into(), total))
}
