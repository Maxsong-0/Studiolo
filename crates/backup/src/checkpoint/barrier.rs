//! 跨 owner 屏障：固定顺序、有界等待、逐项 present/absent。
//!
//! 这里只协调，不持有任何 owner 的连接。每个 owner 自己声明排空证明与
//! 已验证的导出票据；协调器只核对事实，不替 owner 断言「应该已经好了」。

use super::types::{OwnerRoster, SealError, SealTicket, TransferSource};
use studiolo_core::checkpoint::{
    CheckpointError, CheckpointOwner, CheckpointParticipant, FreezeReceipt, FreezeRequest,
    InstantMillis, OwnerAssembly, OwnerWatermark, SealedOwnerCheckpoint,
};

/// 屏障需要的单调时钟；测试注入假时钟以驱动有界等待。
pub trait BarrierClock {
    /// 返回当前时刻。
    fn now(&self) -> InstantMillis;
}

/// 屏障与封存的失败模式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BarrierError {
    /// 来自某个 owner 的协调失败。
    Owner(CheckpointError),
    /// 封存票据本身不合法。
    Seal(SealError),
    /// Broker 没有持有唯一租约，不能开始冻结。
    BrokerLeaseNotSole,
    /// Broker 仍有未决操作，`unresolved=false` 是必要条件。
    BrokerUnresolved,
    /// 导出票据里缺少某个已装配 owner 的水位。
    WatermarkMissing {
        /// 缺少水位的 owner。
        owner: CheckpointOwner,
    },
    /// 时钟在冻结过程中回退，等待预算无法判定。
    ClockWentBackwards,
}

impl std::fmt::Display for BarrierError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Owner(error) => write!(formatter, "跨 owner 协调失败：{error}"),
            Self::Seal(error) => write!(formatter, "封存票据不合法：{error}"),
            Self::BrokerLeaseNotSole => formatter.write_str("Broker 租约不唯一"),
            Self::BrokerUnresolved => formatter.write_str("Broker 仍有未决操作"),
            Self::WatermarkMissing { owner } => {
                write!(formatter, "缺少 owner {} 的水位", owner.as_str())
            }
            Self::ClockWentBackwards => formatter.write_str("冻结期间本地时钟回退"),
        }
    }
}

impl std::error::Error for BarrierError {}

impl From<CheckpointError> for BarrierError {
    fn from(error: CheckpointError) -> Self {
        Self::Owner(error)
    }
}

impl From<SealError> for BarrierError {
    fn from(error: SealError) -> Self {
        Self::Seal(error)
    }
}

/// Broker 在开始冻结前必须成立的前置条件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrokerPrecondition {
    /// 是否持有唯一 canonical 写者租约。
    pub sole_writer_lease: bool,
    /// 是否仍有未决操作。
    pub unresolved: bool,
}

impl BrokerPrecondition {
    fn check(self) -> Result<(), BarrierError> {
        if !self.sole_writer_lease {
            return Err(BarrierError::BrokerLeaseNotSole);
        }
        if self.unresolved {
            return Err(BarrierError::BrokerUnresolved);
        }
        Ok(())
    }
}

/// 按固定顺序登记的六个参与者。
pub struct CrossOwnerBarrier<'a> {
    participants: Vec<&'a dyn CheckpointParticipant>,
}

impl<'a> CrossOwnerBarrier<'a> {
    /// 登记参与者。六个 owner 必须全部登记，未装配的也要登记为 Absent。
    pub fn assemble(
        participants: impl IntoIterator<Item = &'a dyn CheckpointParticipant>,
    ) -> Result<Self, BarrierError> {
        let participants: Vec<_> = participants.into_iter().collect();
        for (index, owner) in CheckpointOwner::FREEZE_ORDER.iter().enumerate() {
            let found = participants
                .iter()
                .filter(|participant| participant.owner() == *owner)
                .count();
            if found == 0 {
                return Err(CheckpointError::OwnerMissing { owner: *owner }.into());
            }
            if found > 1 {
                return Err(CheckpointError::OwnerDuplicated { owner: *owner }.into());
            }
            let Some(candidate) = participants.get(index) else {
                return Err(CheckpointError::OwnerMissing { owner: *owner }.into());
            };
            if candidate.owner() != *owner {
                return Err(CheckpointError::OwnerOutOfOrder { owner: *owner }.into());
            }
        }
        Ok(Self { participants })
    }

    /// 按固定顺序逐个冻结；任一 owner 未排空或超时即全部解冻。
    pub fn freeze<C: BarrierClock>(
        &self,
        request: &FreezeRequest,
        broker: BrokerPrecondition,
        clock: &C,
    ) -> Result<FrozenWindow<'_, 'a>, BarrierError> {
        broker.check()?;
        let mut window = FrozenWindow {
            participants: &self.participants,
            frozen: Vec::new(),
            receipts: Vec::new(),
            roster_entries: Vec::new(),
            thawed: false,
        };
        for participant in &self.participants {
            let owner = participant.owner();
            let assembly = participant.assembly();
            window.roster_entries.push((owner, assembly));
            if assembly == OwnerAssembly::Absent {
                continue;
            }
            let started = clock.now();
            let receipt = match participant.freeze(request) {
                Ok(receipt) => receipt,
                Err(error) => {
                    window.thaw();
                    return Err(error.into());
                }
            };
            let finished = clock.now();
            let elapsed = match finished.millis().checked_sub(started.millis()) {
                Some(elapsed) if elapsed >= 0 => elapsed,
                _ => {
                    window.thaw();
                    return Err(BarrierError::ClockWentBackwards);
                }
            };
            window.frozen.push(*participant);
            if elapsed > i64::from(request.budget.millis()) {
                window.thaw();
                return Err(CheckpointError::FreezeTimeout { owner }.into());
            }
            if receipt.owner != owner {
                window.thaw();
                return Err(CheckpointError::OwnerOutOfOrder { owner }.into());
            }
            if !receipt.drain.is_drained() {
                window.thaw();
                return Err(CheckpointError::NotDrained { owner }.into());
            }
            window.receipts.push(receipt);
        }
        Ok(window)
    }
}

/// 已冻结的协调窗口；离开作用域时按逆序解冻。
pub struct FrozenWindow<'barrier, 'a> {
    participants: &'barrier [&'a dyn CheckpointParticipant],
    frozen: Vec<&'a dyn CheckpointParticipant>,
    receipts: Vec<FreezeReceipt>,
    roster_entries: Vec<(CheckpointOwner, OwnerAssembly)>,
    thawed: bool,
}

impl FrozenWindow<'_, '_> {
    /// 返回逐项标记 present/absent 的清单。
    pub fn roster(&self) -> Result<OwnerRoster, BarrierError> {
        Ok(OwnerRoster::new(self.roster_entries.iter().copied())?)
    }

    /// 返回各已装配 owner 的冻结回执。
    #[must_use]
    pub fn receipts(&self) -> &[FreezeReceipt] {
        &self.receipts
    }

    /// 让每个已装配 owner 导出自己经过验证的 checkpoint。
    pub fn export(
        &self,
        request: &FreezeRequest,
    ) -> Result<Vec<SealedOwnerCheckpoint>, BarrierError> {
        let mut sealed = Vec::new();
        for participant in self.participants {
            if participant.assembly() == OwnerAssembly::Absent {
                continue;
            }
            let owner = participant.owner();
            let checkpoint = participant.export(request)?;
            if checkpoint.owner != owner {
                return Err(CheckpointError::OwnerOutOfOrder { owner }.into());
            }
            if !checkpoint.is_verified() {
                return Err(CheckpointError::Unverified { owner }.into());
            }
            sealed.push(checkpoint);
        }
        Ok(sealed)
    }

    /// 导出并组装封存票据；Broker 水位仍有未决即拒绝。
    pub fn seal(
        &mut self,
        request: &FreezeRequest,
        transfer_source: TransferSource,
        content_cutoff: InstantMillis,
    ) -> Result<SealTicket, BarrierError> {
        let roster = self.roster()?;
        let owners = self.export(request)?;
        for (owner, assembly) in roster.entries() {
            if assembly == OwnerAssembly::Present
                && !owners.iter().any(|sealed| sealed.owner == owner)
            {
                return Err(BarrierError::WatermarkMissing { owner });
            }
        }
        if let Some(SealedOwnerCheckpoint {
            watermark: OwnerWatermark::Broker { unresolved, .. },
            ..
        }) = owners
            .iter()
            .find(|sealed| sealed.owner == CheckpointOwner::Broker)
            && *unresolved
        {
            return Err(BarrierError::BrokerUnresolved);
        }
        Ok(SealTicket {
            checkpoint_id: request.checkpoint_id.clone(),
            roster,
            owners,
            transfer_source,
            content_cutoff,
        })
    }

    /// 按逆序解冻；失败路径与成功路径都会走到这里。
    pub fn thaw(&mut self) {
        if self.thawed {
            return;
        }
        self.thawed = true;
        // 解冻失败无法回滚已冻结的其他 owner，也不能伪造成功；
        // 这里逐个尝试并继续，真实状态由各 owner 自己的健康检查暴露。
        for participant in self.frozen.iter().rev() {
            let _ = participant.thaw();
        }
    }
}

impl Drop for FrozenWindow<'_, '_> {
    fn drop(&mut self) {
        self.thaw();
    }
}
