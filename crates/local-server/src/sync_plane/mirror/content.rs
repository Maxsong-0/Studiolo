//! 冻结版本的正文来源与外 stage 出口：canonical 侧经 `coordinator::sync_bridge` 的窄接缝整读，
//! 镜像侧经视图逐帧流式读并在结束时核摘要；两侧打开时都核对 sha256／bytes 与冻结版本一致。
//!
//! 本文件不打开路径：视图里的文件由 `mirror.rs` 借 `stage.rs` 的助手打开后交进来。

use super::{MirrorView, PlaneError, descend, open_view, regular_at, split, stage};
use crate::sync_plane::SyncPlane;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::io::Errno;
use sha2::{Digest, Sha256};
use studiolo_core::VaultPath;
use studiolo_sync_bridge::executor::{MirrorContent, MirrorObjectSink, ObjectId, TransferFault};
use studiolo_sync_bridge::{FileVersion, Side};

/// canonical 侧单次整读的上限；与执行器单对象上限一致。
const CANONICAL_MAX_BYTES: u64 = 256 * 1024 * 1024;
/// 流式读的固定帧长。
const FRAME_BYTES: usize = 64 * 1024;

/// 冻结版本的正文来源：canonical 侧经 `coordinator::sync_bridge` 读，镜像侧经视图读；
/// 打开时核对 sha256／bytes 与冻结版本一致，不一致即 `ContentMismatch`。
pub struct PlaneContent<'a> {
    view: &'a MirrorView,
    plane: &'a SyncPlane,
    open: Option<OpenSource>,
}

enum OpenSource {
    /// canonical 侧：整段正文已核对，按偏移分帧交出。
    Canonical { bytes: Vec<u8>, offset: usize },
    /// 镜像侧：逐帧读、累计摘要，结束时核对。
    Mirror {
        file: OwnedFd,
        version: FileVersion,
        hasher: Sha256,
        read: u64,
    },
}

impl<'a> PlaneContent<'a> {
    /// 组装正文来源。
    #[must_use]
    pub fn new(plane: &'a SyncPlane, view: &'a MirrorView) -> Self {
        Self {
            view,
            plane,
            open: None,
        }
    }

    fn open_canonical(
        &self,
        path: &VaultPath,
        version: &FileVersion,
    ) -> Result<OpenSource, TransferFault> {
        let read = self
            .plane
            .workbench()
            .bridge_read(path, CANONICAL_MAX_BYTES)
            .map_err(|_| TransferFault::Unavailable)?;
        let bytes = read.content;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        if bytes.len() as u64 != version.bytes || digest != version.sha256 {
            return Err(TransferFault::ContentMismatch);
        }
        Ok(OpenSource::Canonical { bytes, offset: 0 })
    }

    fn open_mirror(
        &self,
        path: &VaultPath,
        version: &FileVersion,
    ) -> Result<OpenSource, TransferFault> {
        let (parents, name) = split(path);
        let root = open_view(self.view).map_err(|_| TransferFault::Unavailable)?;
        let parent = descend(root, &parents, false)
            .map_err(|_| TransferFault::Unavailable)?
            .ok_or(TransferFault::SourceMissing)?;
        let (file, bytes) = regular_at(parent.as_fd(), name)
            .map_err(|_| TransferFault::Unavailable)?
            .ok_or(TransferFault::SourceMissing)?;
        if bytes != version.bytes {
            return Err(TransferFault::ContentMismatch);
        }
        Ok(OpenSource::Mirror {
            file,
            version: version.clone(),
            hasher: Sha256::new(),
            read: 0,
        })
    }
}

impl MirrorContent for PlaneContent<'_> {
    fn open(
        &mut self,
        side: Side,
        path: &VaultPath,
        version: &FileVersion,
    ) -> Result<(), TransferFault> {
        self.open = None;
        let source = match side {
            Side::Canonical => self.open_canonical(path, version)?,
            Side::Mirror => self.open_mirror(path, version)?,
        };
        self.open = Some(source);
        Ok(())
    }

    fn next_frame(&mut self, buffer: &mut [u8]) -> Result<usize, TransferFault> {
        match self.open.as_mut().ok_or(TransferFault::Unavailable)? {
            OpenSource::Canonical { bytes, offset } => {
                let count = buffer.len().min(bytes.len() - *offset);
                buffer[..count].copy_from_slice(&bytes[*offset..*offset + count]);
                *offset += count;
                Ok(count)
            }
            OpenSource::Mirror {
                file,
                version,
                hasher,
                read,
            } => {
                let count =
                    read_frame(file.as_fd(), buffer).map_err(|_| TransferFault::Unavailable)?;
                if count == 0 {
                    let digest: [u8; 32] = hasher.clone().finalize().into();
                    return if *read < version.bytes {
                        Err(TransferFault::Truncated)
                    } else if digest == version.sha256 {
                        Ok(0)
                    } else {
                        Err(TransferFault::ContentMismatch)
                    };
                }
                *read += count as u64;
                if *read > version.bytes {
                    return Err(TransferFault::ContentMismatch);
                }
                hasher.update(&buffer[..count]);
                Ok(count)
            }
        }
    }
}

/// 外 stage 的接收出口：`begin` 建临时件，`accept` 追加，`finish` 按地址 renameat 封口。
pub struct StageSink<'a> {
    stage: &'a mut stage::StageStore,
}

impl<'a> StageSink<'a> {
    /// 绑定到已打开的外 stage。
    #[must_use]
    pub fn new(stage: &'a mut stage::StageStore) -> Self {
        Self { stage }
    }
}

impl MirrorObjectSink for StageSink<'_> {
    fn begin(&mut self) -> Result<(), TransferFault> {
        self.stage.begin().map_err(|_| TransferFault::Unavailable)
    }

    fn accept(&mut self, frame: &[u8]) -> Result<(), TransferFault> {
        self.stage
            .accept(frame)
            .map_err(|_| TransferFault::Unavailable)
    }

    fn finish(&mut self, id: ObjectId) -> Result<(), TransferFault> {
        self.stage
            .finish(id)
            .map_err(|_| TransferFault::Unavailable)
    }

    fn discard(&mut self) -> Result<(), TransferFault> {
        self.stage.discard().map_err(|_| TransferFault::Unavailable)
    }
}

/// 读一帧；被信号打断就重试。
fn read_frame(file: BorrowedFd<'_>, buffer: &mut [u8]) -> rustix::io::Result<usize> {
    loop {
        match rustix::io::read(file, &mut *buffer) {
            Err(Errno::INTR) => {}
            result => return result,
        }
    }
}

/// 流式读完整个文件：累计 sha256 与字节数；给了 `sink` 就同时逐帧送进外 stage。
/// 超过 `limit` 或读失败返回 `Observation`；stage 拒绝按其错误返回。
pub(super) fn stream(
    file: BorrowedFd<'_>,
    limit: u64,
    mut sink: Option<&mut stage::StageStore>,
) -> Result<([u8; 32], u64), PlaneError> {
    let mut buffer = vec![0_u8; FRAME_BYTES];
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    loop {
        let count = read_frame(file, &mut buffer).map_err(|_| PlaneError::Observation)?;
        if count == 0 {
            return Ok((hasher.finalize().into(), total));
        }
        total = total
            .checked_add(count as u64)
            .filter(|read| *read <= limit)
            .ok_or(PlaneError::Observation)?;
        if let Some(stage) = sink.as_deref_mut() {
            stage.accept(&buffer[..count])?;
        }
        hasher.update(&buffer[..count]);
    }
}
