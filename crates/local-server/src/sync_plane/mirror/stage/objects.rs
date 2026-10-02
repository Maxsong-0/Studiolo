//! 已封口对象的核验、实际磁盘预算与安装副本；只借 stage 的固定 IO 助手。

use super::super::{
    OBJECT_MAX_BYTES, create_at, entry_names, flush, hex, open_regular_at, read_fully, remove_at,
    write_fully,
};
use super::StageStore;
use crate::sync_plane::PlaneError;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use sha2::{Digest, Sha256};
use studiolo_sync_bridge::executor::{CasLimits, ObjectId};

const INSTALLING_FILE: &str = "installing.partial";
const FRAME_BYTES: usize = 64 * 1024;

impl StageStore {
    /// 核验已登记来源后有界整读；大对象必须走 `verify_preserved` 与流式来源。
    ///
    /// # Errors
    /// 无保全索引、字节数或摘要不符、超出整读预算一律 `Storage`。
    pub fn read_preserved(
        &self,
        id: ObjectId,
        bytes: u64,
        max_bytes: usize,
    ) -> Result<Vec<u8>, PlaneError> {
        let (file, length) = self.open_preserved(id, bytes)?;
        if length > max_bytes {
            return Err(PlaneError::Storage);
        }
        let content = read_fully(file.as_fd(), length).map_err(|_| PlaneError::Storage)?;
        let actual: [u8; 32] = Sha256::digest(&content).into();
        if actual != id.digest() {
            return Err(PlaneError::Storage);
        }
        Ok(content)
    }

    /// 逐帧核验已登记来源；重放不得拿损坏或未登记的孤儿重建原请求。
    ///
    /// # Errors
    /// 无索引、对象缺失、字节数或摘要不符一律 `Storage`。
    pub fn verify_preserved(&self, id: ObjectId, bytes: u64) -> Result<(), PlaneError> {
        let (file, _) = self.open_preserved(id, bytes)?;
        verify(file.as_fd(), bytes, id, None)
    }

    fn open_preserved(&self, id: ObjectId, bytes: u64) -> Result<(OwnedFd, usize), PlaneError> {
        let name = hex(id.digest());
        if !self
            .index
            .preserved
            .iter()
            .any(|entry| entry.object == name && entry.bytes == bytes)
        {
            return Err(PlaneError::Storage);
        }
        let (file, length) =
            open_regular_at(self.objects.as_fd(), &name, self.euid, OBJECT_MAX_BYTES)
                .map_err(|_| PlaneError::Storage)?
                .ok_or(PlaneError::Storage)?;
        if length as u64 != bytes {
            return Err(PlaneError::Storage);
        }
        Ok((file, length))
    }

    /// 所有实际对象（含孤儿和临时件）都计入固定 4096 件／512 MiB 预算。
    pub(super) fn capacity(
        &self,
        extra_bytes: u64,
        extra_entries: usize,
    ) -> Result<u64, PlaneError> {
        let limits = CasLimits::internal();
        let names = entry_names(self.objects.as_fd()).map_err(|_| PlaneError::Storage)?;
        if names.len().saturating_add(extra_entries) > limits.max_stage_entries {
            return Err(PlaneError::Storage);
        }
        let mut total = extra_bytes;
        for name in names {
            let text = name.to_str().ok_or(PlaneError::Storage)?;
            let (_, length) =
                open_regular_at(self.objects.as_fd(), text, self.euid, OBJECT_MAX_BYTES)
                    .map_err(|_| PlaneError::Storage)?
                    .ok_or(PlaneError::Storage)?;
            total = total
                .checked_add(length as u64)
                .ok_or(PlaneError::Storage)?;
            if total > limits.max_stage_bytes {
                return Err(PlaneError::Storage);
            }
        }
        limits
            .max_stage_bytes
            .checked_sub(total)
            .ok_or(PlaneError::Storage)
    }

    /// 安装只消费未引用的对象；已引用对象先流式复制为独立暂存件，不建硬链接。
    pub(in crate::sync_plane::mirror) fn install_source(
        &mut self,
        id: ObjectId,
    ) -> Result<String, PlaneError> {
        if self.receiving.is_some() {
            return Err(PlaneError::Storage);
        }
        let name = hex(id.digest());
        if !self.index.contains(&name) {
            return Ok(name);
        }
        let objects = self.objects.as_fd();
        remove_at(objects, INSTALLING_FILE).map_err(|_| PlaneError::Storage)?;
        let (source, length) = open_regular_at(objects, &name, self.euid, OBJECT_MAX_BYTES)
            .map_err(|_| PlaneError::Storage)?
            .ok_or(PlaneError::Storage)?;
        self.capacity(length as u64, 1)?;
        let (output, _) =
            create_at(objects, INSTALLING_FILE, true).map_err(|_| PlaneError::Storage)?;
        let copied = verify(source.as_fd(), length as u64, id, Some(output.as_fd()))
            .and_then(|()| flush(output.as_fd()).map_err(|_| PlaneError::Storage));
        drop(output);
        if copied.is_err() {
            let _ = remove_at(objects, INSTALLING_FILE);
        }
        copied?;
        flush(objects).map_err(|_| PlaneError::Storage)?;
        Ok(INSTALLING_FILE.to_owned())
    }
}

/// 只操作已打开 FD；逐帧核字节数和摘要，可选地同时复制进固定安装暂存件。
pub(super) fn verify(
    file: BorrowedFd<'_>,
    bytes: u64,
    id: ObjectId,
    output: Option<BorrowedFd<'_>>,
) -> Result<(), PlaneError> {
    let mut buffer = [0u8; FRAME_BYTES];
    let mut digest = Sha256::new();
    let mut read = 0_u64;
    loop {
        let count = match rustix::io::read(file, &mut buffer) {
            Err(rustix::io::Errno::INTR) => continue,
            result => result.map_err(|_| PlaneError::Storage)?,
        };
        if count == 0 {
            break;
        }
        read = read.checked_add(count as u64).ok_or(PlaneError::Storage)?;
        if read > bytes {
            return Err(PlaneError::Storage);
        }
        digest.update(&buffer[..count]);
        if let Some(target) = output {
            write_fully(target, &buffer[..count]).map_err(|_| PlaneError::Storage)?;
        }
    }
    let digest: [u8; 32] = digest.finalize().into();
    if read != bytes || digest != id.digest() {
        return Err(PlaneError::Storage);
    }
    Ok(())
}
