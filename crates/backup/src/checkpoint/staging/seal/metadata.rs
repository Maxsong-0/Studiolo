//! 清单有总大小上限；序列化过程中即施加预算，不先构造无限大的 Vec。
use super::super::{
    Directory, MAX_METADATA, Result, StagingError, check_deadline, digest, io::Regular,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{io::Write, time::Instant};

pub(super) fn write(
    directory: &Directory,
    name: &str,
    value: &impl Serialize,
    deadline: Instant,
) -> Result<String> {
    let mut buffer = Limited(Vec::new());
    serde_json::to_writer(&mut buffer, value).map_err(|_| StagingError::Limit)?;
    check_deadline(deadline)?;
    let mut file = Regular::create(directory, name, buffer.0.len() as u64)?;
    for chunk in buffer.0.chunks(256 * 1024) {
        check_deadline(deadline)?;
        file.write(chunk)?;
    }
    file.finish()?;
    directory.sync()?;
    Ok(digest(&buffer.0))
}
pub(super) fn read<T: DeserializeOwned>(
    directory: &Directory,
    name: &str,
    deadline: Instant,
) -> Result<(T, String, u64)> {
    let mut file = Regular::open(directory, name)?;
    file.require_readonly()?;
    if file.len() > MAX_METADATA {
        return Err(StagingError::Limit);
    }
    let mut bytes = Vec::new();
    let mut chunk = vec![0; 256 * 1024];
    loop {
        check_deadline(deadline)?;
        let count = file.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        if bytes.len() as u64 + count as u64 > file.len() {
            return Err(StagingError::Changed);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    if bytes.len() as u64 != file.len() {
        return Err(StagingError::Changed);
    }
    file.check(directory, name)?;
    let result = serde_json::from_slice(&bytes).map_err(|_| StagingError::Manifest)?;
    check_deadline(deadline)?;
    Ok((result, digest(&bytes), bytes.len() as u64))
}
struct Limited(Vec<u8>);
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len() as u64 + bytes.len() as u64 > MAX_METADATA {
            return Err(std::io::Error::other("封存清单超过预算"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
