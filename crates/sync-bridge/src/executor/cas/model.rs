//! MirrorCAS 的封闭 DTO：内容地址、内部工程上限、拒绝原因与占用统计。

use super::super::transfer::TransferFault;
use crate::Digest;

/// 内容地址；只能由已完整摘要的字节或冻结证据铸出。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectId(Digest);

impl ObjectId {
    /// 由冻结证据里的 SHA-256 构造预期地址。
    pub fn from_digest(digest: Digest) -> Self {
        Self(digest)
    }
    /// 原始 SHA-256。
    pub fn digest(&self) -> Digest {
        self.0
    }
}

/// 内部工程上限；不得冒充服务端配额。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CasLimits {
    /// 单对象上限，默认 256 MiB。
    pub max_object_bytes: u64,
    /// stage 合计上限（含孤儿），默认 512 MiB。
    pub max_stage_bytes: u64,
    /// stage 条目上限（含孤儿），默认 64。
    pub max_stage_entries: usize,
    /// 同时接收中的对象上限，默认 1。
    pub max_receiving: usize,
    /// 同时 Prepared 的对象上限，默认 1。
    pub max_prepared: usize,
    /// 固定帧上限，默认 64 KiB。
    pub frame_bytes: usize,
}

impl CasLimits {
    /// 冻结的内部默认值。
    pub fn internal() -> Self {
        Self {
            max_object_bytes: 256 * 1024 * 1024,
            max_stage_bytes: 512 * 1024 * 1024,
            max_stage_entries: 64,
            max_receiving: 1,
            max_prepared: 1,
            frame_bytes: 64 * 1024,
        }
    }
}

/// MirrorCAS 拒绝原因闭集。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CasFault {
    /// 声明字节数超过单对象上限。
    ObjectTooLarge,
    /// stage 条目数已达上限。
    StageFull,
    /// stage 合计字节（含孤儿）会超过上限。
    StageBytesExceeded,
    /// 单帧超过固定帧上限。
    FrameTooLarge,
    /// 已有接收中的对象。
    ReceiveInProgress,
    /// 当前没有接收中的对象。
    NoReceiveInProgress,
    /// 唯一 Prepared 槽已被占用。
    PreparedInUse,
    /// 当前 Prepared 槽为空。
    NoPrepared,
    /// 实际字节数与声明不符。
    DeclaredBytesMismatch,
    /// 实际内容地址与预期不符。
    AddressMismatch,
    /// 账本里没有该地址。
    UnknownObject,
    /// 对象仍被引用，不能丢弃。
    StillReferenced,
    /// 精确整数统计不能表示。
    Overflow,
    /// 受控镜像 sink 失败。
    Sink(TransferFault),
}

/// stage 当前占用，孤儿一并计入。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StageUsage {
    /// 已封口条目数（含孤儿）。
    pub entries: usize,
    /// 已封口字节合计（含孤儿）。
    pub bytes: u64,
    /// 无引用条目数。
    pub orphans: usize,
    /// 无引用字节合计。
    pub orphan_bytes: u64,
}
