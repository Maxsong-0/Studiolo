//! MirrorCAS：镜像侧对象按内容寻址的有界暂存账本。
//!
//! 这里的上限是**本实现的内部工程上限**，不是 Obsidian Sync Plus 的服务承诺，
//! 也不能据此对用户宣称任何配额。账本只记地址、字节与引用；真实字节由 sink 拥有。

mod model;

pub use model::{CasFault, CasLimits, ObjectId, StageUsage};

use super::transfer::MirrorObjectSink;
use crate::Digest;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
struct Entry {
    bytes: u64,
    referenced: bool,
}

struct Receiving {
    expected: Option<ObjectId>,
    declared: u64,
    written: u64,
    hasher: Sha256,
}

/// 有界内容寻址暂存；不打开文件、不删除任何东西，只维护账本与唯一槽位。
pub struct MirrorCas {
    limits: CasLimits,
    entries: BTreeMap<ObjectId, Entry>,
    receiving: Option<Receiving>,
    prepared: Option<ObjectId>,
    bytes: u64,
}

impl MirrorCas {
    /// 用给定内部上限建立空账本。
    pub fn new(limits: CasLimits) -> Self {
        Self {
            limits,
            entries: BTreeMap::new(),
            receiving: None,
            prepared: None,
            bytes: 0,
        }
    }
    /// 当前内部上限。
    pub fn limits(&self) -> CasLimits {
        self.limits
    }
    /// 当前占用，孤儿一并计入。
    pub fn usage(&self) -> StageUsage {
        let mut usage = StageUsage {
            entries: self.entries.len(),
            bytes: self.bytes,
            ..StageUsage::default()
        };
        for entry in self.entries.values().filter(|e| !e.referenced) {
            usage.orphans += 1;
            usage.orphan_bytes = usage.orphan_bytes.saturating_add(entry.bytes);
        }
        usage
    }
    /// 是否正有一个对象在接收中。
    pub fn is_receiving(&self) -> bool {
        self.receiving.is_some()
    }
    /// 账本是否已有该地址。
    pub fn contains(&self, id: ObjectId) -> bool {
        self.entries.contains_key(&id)
    }
    /// 当前 Prepared 地址。
    pub fn prepared(&self) -> Option<ObjectId> {
        self.prepared
    }
    /// 当前无引用地址列表。
    pub fn orphans(&self) -> Vec<ObjectId> {
        self.entries
            .iter()
            .filter(|(_, e)| !e.referenced)
            .map(|(id, _)| *id)
            .collect()
    }

    /// 先按声明字节数占额度，再让 sink 开始接收；额度不够时 sink 不会被打开。
    pub fn begin_receive(
        &mut self,
        declared_bytes: u64,
        expected: Option<ObjectId>,
        sink: &mut dyn MirrorObjectSink,
    ) -> Result<(), CasFault> {
        if self.receiving.is_some() || self.limits.max_receiving == 0 {
            return Err(CasFault::ReceiveInProgress);
        }
        if declared_bytes > self.limits.max_object_bytes {
            return Err(CasFault::ObjectTooLarge);
        }
        let known = expected.is_some_and(|id| self.entries.contains_key(&id));
        if !known {
            if self.entries.len() >= self.limits.max_stage_entries {
                return Err(CasFault::StageFull);
            }
            let projected = self
                .bytes
                .checked_add(declared_bytes)
                .ok_or(CasFault::Overflow)?;
            if projected > self.limits.max_stage_bytes {
                return Err(CasFault::StageBytesExceeded);
            }
        }
        sink.begin().map_err(CasFault::Sink)?;
        self.receiving = Some(Receiving {
            expected,
            declared: declared_bytes,
            written: 0,
            hasher: Sha256::new(),
        });
        Ok(())
    }

    /// 接收一帧；超过固定帧上限或超出声明字节数一律拒绝。
    pub fn accept_frame(
        &mut self,
        frame: &[u8],
        sink: &mut dyn MirrorObjectSink,
    ) -> Result<(), CasFault> {
        if frame.len() > self.limits.frame_bytes {
            return Err(CasFault::FrameTooLarge);
        }
        let receiving = self
            .receiving
            .as_mut()
            .ok_or(CasFault::NoReceiveInProgress)?;
        let written = receiving
            .written
            .checked_add(frame.len() as u64)
            .ok_or(CasFault::Overflow)?;
        if written > receiving.declared {
            return Err(CasFault::DeclaredBytesMismatch);
        }
        sink.accept(frame).map_err(CasFault::Sink)?;
        receiving.hasher.update(frame);
        receiving.written = written;
        Ok(())
    }

    /// 校验字节数与内容地址后封口；地址不符不入账。
    pub fn finish_receive(
        &mut self,
        sink: &mut dyn MirrorObjectSink,
    ) -> Result<ObjectId, CasFault> {
        let receiving = self.receiving.take().ok_or(CasFault::NoReceiveInProgress)?;
        if receiving.written != receiving.declared {
            sink.discard().map_err(CasFault::Sink)?;
            return Err(CasFault::DeclaredBytesMismatch);
        }
        let digest: Digest = receiving.hasher.finalize().into();
        let id = ObjectId::from_digest(digest);
        if receiving.expected.is_some_and(|expected| expected != id) {
            sink.discard().map_err(CasFault::Sink)?;
            return Err(CasFault::AddressMismatch);
        }
        sink.finish(id).map_err(CasFault::Sink)?;
        if !self.entries.contains_key(&id) {
            self.bytes = self
                .bytes
                .checked_add(receiving.declared)
                .ok_or(CasFault::Overflow)?;
            self.entries.insert(
                id,
                Entry {
                    bytes: receiving.declared,
                    referenced: false,
                },
            );
        }
        Ok(id)
    }

    /// 放弃接收中的对象；账本不变。
    pub fn abort_receive(&mut self, sink: &mut dyn MirrorObjectSink) -> Result<(), CasFault> {
        self.receiving.take().ok_or(CasFault::NoReceiveInProgress)?;
        sink.discard().map_err(CasFault::Sink)
    }

    /// 占用唯一 Prepared 槽并把对象标为已引用。
    pub fn prepare(&mut self, id: ObjectId) -> Result<(), CasFault> {
        if self.prepared.is_some() || self.limits.max_prepared == 0 {
            return Err(CasFault::PreparedInUse);
        }
        let entry = self.entries.get_mut(&id).ok_or(CasFault::UnknownObject)?;
        entry.referenced = true;
        self.prepared = Some(id);
        Ok(())
    }

    /// 释放 Prepared 槽；`keep` 为真表示对象仍须保全，否则变回孤儿。
    pub fn release_prepared(&mut self, keep: bool) -> Result<ObjectId, CasFault> {
        let id = self.prepared.take().ok_or(CasFault::NoPrepared)?;
        if !keep {
            let entry = self.entries.get_mut(&id).ok_or(CasFault::UnknownObject)?;
            entry.referenced = false;
        }
        Ok(id)
    }

    /// 从账本移除一个无引用对象；已引用对象拒绝移除。
    pub fn forget(&mut self, id: ObjectId) -> Result<(), CasFault> {
        let entry = self.entries.get(&id).ok_or(CasFault::UnknownObject)?;
        if entry.referenced || self.prepared == Some(id) {
            return Err(CasFault::StillReferenced);
        }
        let bytes = entry.bytes;
        self.entries.remove(&id);
        self.bytes = self.bytes.checked_sub(bytes).ok_or(CasFault::Overflow)?;
        Ok(())
    }
}
