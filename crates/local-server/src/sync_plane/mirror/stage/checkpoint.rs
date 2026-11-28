//! 平面租约窗口内封存保全索引和全部登记对象，不执行孤儿回收或保留期裁剪。
use super::super::{checkpoint_io as io, *};
use super::{StageStore, object};
use rustix::fd::{AsFd, BorrowedFd};
use std::time::Instant;
use studiolo_sync_bridge::executor::{CasLimits, ObjectId};

impl StageStore {
    pub(in crate::sync_plane) fn checkpoint_verify(
        &self,
        until: Instant,
    ) -> Result<Vec<u8>, PlaneError> {
        io::deadline(until)?;
        io::root(&self.root, self.stage.as_fd(), self.euid)?;
        let objects = private_directory_at(self.stage.as_fd(), OBJECTS_DIR, self.euid)?;
        io::same(
            &io::metadata(objects.as_fd())?,
            &io::metadata(self.objects.as_fd())?,
        )?;
        if self.receiving.is_some()
            || entry_names(self.stage.as_fd()).map_err(|_| PlaneError::Storage)?
                != [INDEX_FILE, OBJECTS_DIR].map(OsString::from)
        {
            return Err(PlaneError::Storage);
        }
        let (bytes, _) = io::read(self.stage.as_fd(), INDEX_FILE, self.euid, INDEX_MAX_BYTES)?
            .ok_or(PlaneError::Storage)?;
        if StageIndex::decode(&bytes)? != self.index {
            return Err(PlaneError::Storage);
        }
        let names = entry_names(self.objects.as_fd()).map_err(|_| PlaneError::Storage)?;
        let expected: Vec<_> = self
            .index
            .preserved
            .iter()
            .map(|entry| OsString::from(&entry.object))
            .collect();
        if names != expected || names.len() > CasLimits::internal().max_stage_entries {
            return Err(PlaneError::Storage);
        }
        self.capacity(0, 0)?;
        for entry in &self.index.preserved {
            io::deadline(until)?;
            let id = object_id(&entry.object)?;
            let (file, length) = open_regular_at(
                self.objects.as_fd(),
                &entry.object,
                self.euid,
                OBJECT_MAX_BYTES,
            )
            .map_err(|_| PlaneError::Storage)?
            .ok_or(PlaneError::Storage)?;
            if length as u64 != entry.bytes {
                return Err(PlaneError::Storage);
            }
            let before = io::metadata(file.as_fd())?;
            object::verify_until(file.as_fd(), entry.bytes, id, None, Some(until))?;
            let (again, _) = open_regular_at(
                self.objects.as_fd(),
                &entry.object,
                self.euid,
                OBJECT_MAX_BYTES,
            )
            .map_err(|_| PlaneError::Storage)?
            .ok_or(PlaneError::Storage)?;
            io::unchanged(&before, &io::metadata(file.as_fd())?)?;
            io::unchanged(&before, &io::metadata(again.as_fd())?)?;
        }
        io::deadline(until)?;
        Ok(bytes)
    }
    /// 调用方持有真实 plane.lock；目标根已由同一封存窗口核验且尚未有 stage 子目录。
    pub(in crate::sync_plane) fn checkpoint_export(
        &self,
        target: BorrowedFd<'_>,
        until: Instant,
    ) -> Result<(), PlaneError> {
        let index = self.checkpoint_verify(until)?;
        make_directory_at(target, STAGE_DIR, true).map_err(|_| PlaneError::Storage)?;
        let stage = private_directory_at(target, STAGE_DIR, self.euid)?;
        make_directory_at(stage.as_fd(), OBJECTS_DIR, true).map_err(|_| PlaneError::Storage)?;
        let objects = private_directory_at(stage.as_fd(), OBJECTS_DIR, self.euid)?;
        for entry in &self.index.preserved {
            io::deadline(until)?;
            let id = object_id(&entry.object)?;
            let (file, _) = open_regular_at(
                self.objects.as_fd(),
                &entry.object,
                self.euid,
                OBJECT_MAX_BYTES,
            )
            .map_err(|_| PlaneError::Storage)?
            .ok_or(PlaneError::Storage)?;
            let before = io::metadata(file.as_fd())?;
            let output = io::create(objects.as_fd(), &entry.object)?;
            object::verify_until(
                file.as_fd(),
                entry.bytes,
                id,
                Some(output.as_fd()),
                Some(until),
            )?;
            io::sync(output.as_fd())?;
            let (copied, _) =
                open_regular_at(objects.as_fd(), &entry.object, self.euid, OBJECT_MAX_BYTES)
                    .map_err(|_| PlaneError::Storage)?
                    .ok_or(PlaneError::Storage)?;
            io::same(
                &io::metadata(output.as_fd())?,
                &io::metadata(copied.as_fd())?,
            )?;
            object::verify_until(copied.as_fd(), entry.bytes, id, None, Some(until))?;
            io::unchanged(&before, &io::metadata(file.as_fd())?)?;
        }
        io::write(stage.as_fd(), INDEX_FILE, &index, self.euid)?;
        io::sync(objects.as_fd())?;
        io::sync(stage.as_fd())?;
        io::sync(target)?;
        if self.checkpoint_verify(until)? != index {
            return Err(PlaneError::Storage);
        }
        Ok(())
    }
}
fn object_id(name: &str) -> Result<ObjectId, PlaneError> {
    studiolo_core::from_lower_hex(name)
        .map(ObjectId::from_digest)
        .ok_or(PlaneError::Storage)
}
