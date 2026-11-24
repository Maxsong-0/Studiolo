//! 从定格导出的写者副本：逐条核回执在这里、在放开之后做（BUG-050），冻结窗口里不读回执。
//!
//! 核的是同一套代码（[`WriterRecords`]）：正文回执、文件操作回执、维护槽、重放，四样和以前在活的写者上核的
//! 一样。写者的绑定文件里记着各目录当时的身份；副本的目录是新建的，所以比的时候先按 [`Originals`] 换回定格
//! 那一刻的原身份，再逐个比——不降级成「不比」。
use super::{
    audit::{self, BrokerCheckpointAudit, BrokerCheckpointCoverage},
    check_deadline, replays, roots,
};
use crate::{
    BrokerError, VaultUuid, file_operation, history, maintenance,
    records::WriterRecords,
    writer::{Binding, Identity, directory, disk},
};
use cap_std::fs::Dir;
use std::{collections::HashMap, sync::Mutex, time::Instant};
use studiolo_core::checkpoint::{
    CheckpointOwner, OwnerWatermark, SealedOwnerCheckpoint, WalCheckpointProof,
};

/// 副本里每个目录和文件对应定格那一刻的哪一个。拷的时候记下，之后只查。
#[derive(Default)]
pub(super) struct Originals(HashMap<Identity, Identity>);
impl Originals {
    /// 同一个副本节点记两次说明拷乱了，整份失败。
    pub(super) fn record(&mut self, copy: Identity, original: Identity) -> Result<(), BrokerError> {
        match self.0.insert(copy, original) {
            None => Ok(()),
            Some(_) => Err(BrokerError::CaptureChanged),
        }
    }
    /// 没记过的节点不是这次拷出来的（被换过）：拒绝，不拿它现在的身份充数。
    pub(super) fn of(&self, copy: Identity) -> Result<Identity, BrokerError> {
        self.0
            .get(&copy)
            .copied()
            .ok_or(BrokerError::WriterBindingMismatch)
    }
}

/// 从定格导出、已落盘、摘要对得上的写者副本，还没核过回执：没有票据，只能拿去 [`ExportedBroker::verify`]。
#[must_use = "没核过的副本不能算封存"]
pub struct ExportedBroker {
    target: roots::Targets,
    originals: Originals,
    /// 定格那一刻写者的绑定：副本里两份绑定文件都要等于它。
    binding: Binding,
    watermark: OwnerWatermark,
    history_pin: Mutex<Option<history::store::Pin>>,
    maintenance_pin: Mutex<Option<maintenance::store::Pin>>,
    file_operation_pin: Mutex<Option<file_operation::binding::Pin>>,
}
impl ExportedBroker {
    pub(super) fn new(
        target: roots::Targets,
        originals: Originals,
        binding: Binding,
        watermark: OwnerWatermark,
    ) -> Self {
        Self {
            target,
            originals,
            binding,
            watermark,
            history_pin: Mutex::new(None),
            maintenance_pin: Mutex::new(None),
            file_operation_pin: Mutex::new(None),
        }
    }
    /// 在副本上逐条核重放和回执，核完才有票据。可信组合根须用定格的原 FrozenAudit 完整核对事件、offset
    /// 和 hash，不能只接受正整数；核对覆盖已清理 outbox 的永久 ACK。核不过整份作废，副本由暂存层回收。
    pub fn verify(
        self,
        deadline: Instant,
        mut verify: impl FnMut(BrokerCheckpointAudit<'_>, u64, &str) -> Result<(), BrokerError>,
    ) -> Result<(SealedOwnerCheckpoint, BrokerCheckpointCoverage), BrokerError> {
        self.layout()?;
        replays::verify(&self, deadline)?;
        let coverage = audit::verify(&self, deadline, &mut verify)?;
        self.layout()?;
        check_deadline(deadline)?;
        Ok((self.ticket(), coverage))
    }
    /// 副本还是导出的那一份，两份绑定文件、笔记库标识和布局里每个目录都对得上定格那一刻的写者。
    /// 活的写者每次读前读后都核一遍（`Writer::check`）；副本是私有目录，读取都从这里打开的目录句柄走，
    /// 没有会被换掉的活路径，核对前后各核一次。
    fn layout(&self) -> Result<(), BrokerError> {
        self.target.check()?;
        let control = directory(&self.target.vault, ".workbench")?;
        for dir in [&control, &self.target.data] {
            let bytes = disk::read(dir, "broker-binding.json", disk::IDENTITY_LIMIT)?
                .ok_or(BrokerError::WriterBindingMismatch)?;
            if disk::decode::<Binding>(&bytes)? != self.binding {
                return Err(BrokerError::WriterBindingMismatch);
            }
        }
        self.binding.verify_as(
            &self.target.vault,
            &self.target.data,
            self.binding.vault_uuid,
            |copy| self.originals.of(copy),
        )
    }
    fn ticket(&self) -> SealedOwnerCheckpoint {
        SealedOwnerCheckpoint {
            owner: CheckpointOwner::Broker,
            schema_version: 1,
            wal: WalCheckpointProof {
                busy: 0,
                log_frames: 0,
                checkpointed_frames: 0,
            },
            target_synced: true,
            parent_directory_synced: true,
            reopen_verified: true,
            source_binding_stable: true,
            watermark: self.watermark.clone(),
        }
    }
}

impl WriterRecords for ExportedBroker {
    fn root(&self) -> &Dir {
        &self.target.vault
    }
    fn data(&self) -> &Dir {
        &self.target.data
    }
    fn vault_uuid(&self) -> VaultUuid {
        self.binding.vault_uuid
    }
    fn pair_id(&self) -> VaultUuid {
        self.binding.pair_id
    }
    /// 不在每次读前读后重核，见 [`ExportedBroker::layout`]。
    fn check(&self) -> Result<(), BrokerError> {
        Ok(())
    }
    fn bound_identity(&self, dir: &Dir) -> Result<Identity, BrokerError> {
        self.originals.of(Identity::directory(dir)?)
    }
    fn history_pin(&self) -> &Mutex<Option<history::store::Pin>> {
        &self.history_pin
    }
    fn maintenance_pin(&self) -> &Mutex<Option<maintenance::store::Pin>> {
        &self.maintenance_pin
    }
    fn file_operation_pin(&self) -> &Mutex<Option<file_operation::binding::Pin>> {
        &self.file_operation_pin
    }
    /// 定格前后的水位都要求没有未决写入，副本里不会有。
    fn uncertain(&self) -> bool {
        false
    }
    /// 副本里的水位只能是定格那一刻的：多一笔、少一笔都不是同一份。
    fn observe_committed(&self, committed: u64) -> Result<(), BrokerError> {
        match &self.watermark {
            OwnerWatermark::Broker {
                committed_through, ..
            } if *committed_through == committed => Ok(()),
            _ => Err(BrokerError::CorruptWriterState),
        }
    }
}
