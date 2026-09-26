//! `StageStore`：外 stage 的对象落库、保全登记与回收。只调用 `stage.rs` 里的固定 IO 助手，
//! 本文件自己不出现任何打开／删除／改名调用。

use super::{
    INDEX_FILE, INDEX_MAX_BYTES, INDEX_STAGE_FILE, OBJECT_MAX_BYTES, OBJECTS_DIR, PreservedEntry,
    RECEIVING_FILE, STAGE_DIR, StageIndex, caller, create_at, entry_names, flush, hex,
    make_directory_at, open_directory_path, open_regular_at, private_directory,
    private_directory_at, read_fully, remove_at, rename_within, replace_file, write_fully,
};
use crate::sync_plane::PlaneError;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use studiolo_sync_bridge::executor::ObjectId;

/// 已打开的外 stage；持有 stage 与 objects 两个目录 FD、解析后的索引与接收中对象。
pub struct StageStore {
    root: PathBuf,
    stage: OwnedFd,
    objects: OwnedFd,
    euid: u32,
    index: StageIndex,
    receiving: Option<Receiving>,
}

struct Receiving {
    file: OwnedFd,
    hasher: Sha256,
    bytes: usize,
}

impl StageStore {
    /// 打开 `<plane_root>/stage`：核对 0700／本人所有，读索引（失败关闭），回收孤儿。
    ///
    /// # Errors
    /// 目录或索引不满足约束返回 `Storage`。
    pub fn open(plane_root: &Path) -> Result<Self, PlaneError> {
        let euid = caller()?;
        let root = plane_root.join(STAGE_DIR);
        let (stage, stat) = open_directory_path(&root).map_err(|_| PlaneError::Storage)?;
        if !private_directory(&stat, euid) {
            return Err(PlaneError::Storage);
        }
        let objects = private_directory_at(stage.as_fd(), OBJECTS_DIR, euid)?;
        let index = match open_regular_at(stage.as_fd(), INDEX_FILE, euid, INDEX_MAX_BYTES)
            .map_err(|_| PlaneError::Storage)?
        {
            Some((file, length)) => StageIndex::decode(
                &read_fully(file.as_fd(), length).map_err(|_| PlaneError::Storage)?,
            )?,
            None => StageIndex::empty(),
        };
        let store = Self {
            root,
            stage,
            objects,
            euid,
            index,
            receiving: None,
        };
        if store.collect_orphans()? > 0 {
            flush(store.objects.as_fd()).map_err(|_| PlaneError::Storage)?;
        }
        Ok(store)
    }

    /// 管理员显式初始化：`stage/` 与 `stage/objects/` 都不存在时才建，并写入空索引。
    ///
    /// # Errors
    /// 平面根不是本人 0700 目录或任一目录已存在返回 `Storage`。
    pub fn initialize(plane_root: &Path) -> Result<(), PlaneError> {
        let euid = caller()?;
        let (root, stat) = open_directory_path(plane_root).map_err(|_| PlaneError::Storage)?;
        if !private_directory(&stat, euid) {
            return Err(PlaneError::Storage);
        }
        make_directory_at(root.as_fd(), STAGE_DIR, true).map_err(|_| PlaneError::Storage)?;
        let stage = private_directory_at(root.as_fd(), STAGE_DIR, euid)?;
        make_directory_at(stage.as_fd(), OBJECTS_DIR, true).map_err(|_| PlaneError::Storage)?;
        let objects = private_directory_at(stage.as_fd(), OBJECTS_DIR, euid)?;
        let index = StageIndex::empty().encode()?;
        replace_file(stage.as_fd(), INDEX_STAGE_FILE, INDEX_FILE, &index)
            .and_then(|()| flush(objects.as_fd()))
            .and_then(|()| flush(root.as_fd()))
            .map_err(|_| PlaneError::Storage)
    }

    /// 开始接收一个对象：独占创建 `receiving.partial`。上次崩在接收途中留下的同名临时件
    /// 从未封口、不属于任何已登记对象，先清掉再建。
    ///
    /// # Errors
    /// 已有接收中对象或创建失败返回 `Storage`。
    pub fn begin(&mut self) -> Result<(), PlaneError> {
        if self.receiving.is_some() {
            return Err(PlaneError::Storage);
        }
        let objects = self.objects.as_fd();
        let (file, _) = remove_at(objects, RECEIVING_FILE)
            .and_then(|_| create_at(objects, RECEIVING_FILE, true))
            .map_err(|_| PlaneError::Storage)?;
        self.receiving = Some(Receiving {
            file,
            hasher: Sha256::new(),
            bytes: 0,
        });
        Ok(())
    }

    /// 追加一帧到接收中对象；超过单对象上限即拒绝。
    ///
    /// # Errors
    /// 没有接收中对象、超限或写失败返回 `Storage`。
    pub fn accept(&mut self, frame: &[u8]) -> Result<(), PlaneError> {
        let receiving = self.receiving.as_mut().ok_or(PlaneError::Storage)?;
        let bytes = receiving
            .bytes
            .checked_add(frame.len())
            .filter(|total| *total <= OBJECT_MAX_BYTES)
            .ok_or(PlaneError::Storage)?;
        write_fully(receiving.file.as_fd(), frame).map_err(|_| PlaneError::Storage)?;
        receiving.hasher.update(frame);
        receiving.bytes = bytes;
        Ok(())
    }

    /// 按最终地址封口：核对摘要 → fsync → renameat 到 `objects/<hex>` → fsync 目录。
    ///
    /// # Errors
    /// 摘要与地址不符或 IO 失败返回 `Storage`；地址已存在时丢弃临时件并静默成功。
    pub fn finish(&mut self, id: ObjectId) -> Result<(), PlaneError> {
        let receiving = self.receiving.take().ok_or(PlaneError::Storage)?;
        let digest: [u8; 32] = receiving.hasher.finalize().into();
        let objects = self.objects.as_fd();
        if digest != id.digest() {
            remove_at(objects, RECEIVING_FILE).map_err(|_| PlaneError::Storage)?;
            return Err(PlaneError::Storage);
        }
        flush(receiving.file.as_fd()).map_err(|_| PlaneError::Storage)?;
        drop(receiving.file);
        let name = hex(digest);
        let sealed = open_regular_at(objects, &name, self.euid, OBJECT_MAX_BYTES)
            .map_err(|_| PlaneError::Storage)?;
        if sealed.is_some() {
            remove_at(objects, RECEIVING_FILE).map_err(|_| PlaneError::Storage)?;
        } else {
            rename_within(objects, RECEIVING_FILE, &name).map_err(|_| PlaneError::Storage)?;
        }
        flush(objects).map_err(|_| PlaneError::Storage)
    }

    /// 放弃接收中对象并删除临时件。
    ///
    /// # Errors
    /// 没有接收中对象或删除失败返回 `Storage`。
    pub fn discard(&mut self) -> Result<(), PlaneError> {
        drop(self.receiving.take().ok_or(PlaneError::Storage)?);
        remove_at(self.objects.as_fd(), RECEIVING_FILE).map_err(|_| PlaneError::Storage)?;
        Ok(())
    }

    /// 把已封口对象登记为「删除前保全」，进入 7 天保留期；同一地址重复保全以最新为准。
    ///
    /// # Errors
    /// 对象不存在或索引写失败返回 `Storage`。
    pub fn preserve(
        &mut self,
        id: ObjectId,
        plan_sequence: u64,
        original_path: &str,
        now_unix: i64,
    ) -> Result<(), PlaneError> {
        let object = hex(id.digest());
        let (_, length) =
            open_regular_at(self.objects.as_fd(), &object, self.euid, OBJECT_MAX_BYTES)
                .map_err(|_| PlaneError::Storage)?
                .ok_or(PlaneError::Storage)?;
        let mut next = self.index.clone();
        next.upsert(PreservedEntry {
            object,
            bytes: length as u64,
            preserved_at_unix: now_unix,
            plan_sequence,
            original_path: original_path.to_owned(),
        });
        self.save_index(next)
    }

    fn save_index(&mut self, next: StageIndex) -> Result<(), PlaneError> {
        replace_file(
            self.stage.as_fd(),
            INDEX_STAGE_FILE,
            INDEX_FILE,
            &next.encode()?,
        )
        .map_err(|_| PlaneError::Storage)?;
        self.index = next;
        Ok(())
    }

    /// 对象文件路径；只读用途（canonical 侧流式写的附件来源）。
    #[must_use]
    pub fn object_path(&self, id: ObjectId) -> PathBuf {
        self.root.join(OBJECTS_DIR).join(hex(id.digest()))
    }

    /// 对象目录描述符；只给 `mirror::install` 作 renameat 的源目录。
    pub(in crate::sync_plane::mirror) fn objects(&self) -> BorrowedFd<'_> {
        self.objects.as_fd()
    }

    /// 回收：保留期已到且 `plan_sequence` ≤ `retired_through` 的保全对象，以及索引外的孤儿
    /// （接收中除外）。返回回收的对象数。
    ///
    /// # Errors
    /// IO 失败返回 `Storage`。
    pub fn sweep(&mut self, now_unix: i64, retired_through: u64) -> Result<usize, PlaneError> {
        let mut next = self.index.clone();
        let expired = next.take_expired(now_unix, retired_through);
        if !expired.is_empty() {
            self.save_index(next)?;
        }
        let mut removed = 0;
        for object in &expired {
            let gone = remove_at(self.objects.as_fd(), object).map_err(|_| PlaneError::Storage)?;
            removed += usize::from(gone);
        }
        removed += self.collect_orphans()?;
        if removed > 0 {
            flush(self.objects.as_fd()).map_err(|_| PlaneError::Storage)?;
        }
        Ok(removed)
    }

    /// 删掉 `objects/` 里既不在索引也不是接收临时件的名字；返回删掉的个数，不 fsync。
    fn collect_orphans(&self) -> Result<usize, PlaneError> {
        let objects = self.objects.as_fd();
        let mut removed = 0;
        for name in entry_names(objects).map_err(|_| PlaneError::Storage)? {
            let known = name
                .to_str()
                .is_some_and(|text| text == RECEIVING_FILE || self.index.contains(text));
            if !known {
                let gone = remove_at(objects, &name).map_err(|_| PlaneError::Storage)?;
                removed += usize::from(gone);
            }
        }
        Ok(removed)
    }

    /// 当前索引（只读）。
    #[must_use]
    pub fn index(&self) -> &StageIndex {
        &self.index
    }
}
