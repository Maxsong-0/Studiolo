//! 写者旧记录打包的入口（BUG-051、T-235）：只由备份后台线程在不封存的那一轮调，不开放给任何 HTTP 入口。
//! 和写入走同一套许可与锁：一步打完才放开，封存等得到它，它也不会夹在封存中间。
use super::{CanonicalWorkbench, Core};
use crate::{LocalError, bounded_lock, health::RecordUsageProbe};
use std::{sync::PoisonError, time::Instant};
use studiolo_vault_broker::{
    ArchiveCursor, ArchiveCutoff, ArchiveStep, BrokerError, FsVaultBroker, RecordUsage,
};

impl CanonicalWorkbench {
    /// 用量到一半，该打包了；只看内存里最近一次的用量，没有就是还不该。
    pub(crate) fn archive_due(&self) -> bool {
        self.core
            .cached_usage()
            .is_some_and(|usage| usage.half_full())
    }

    /// 打一步。拿不到许可或锁、在关停、写者不在都是「不可用」，调用方下一轮再来。
    pub(crate) fn archive_step(
        &self,
        cutoff: ArchiveCutoff,
        cursor: &mut ArchiveCursor,
    ) -> Result<ArchiveStep, LocalError> {
        let _operation = self
            .core
            .operation()
            .map_err(|()| LocalError::Unavailable)?;
        let mut owner =
            bounded_lock::lock_until(&self.core.owner, Instant::now() + bounded_lock::BRIEF)
                .map_err(|_| LocalError::Unavailable)?;
        let broker = owner.as_mut().ok_or(LocalError::Unavailable)?;
        if self
            .core
            .control
            .lock()
            .map_err(|_| LocalError::Unavailable)?
            .closing
        {
            return Err(LocalError::Unavailable);
        }
        self.core
            .roots
            .revalidate()
            .map_err(|_| LocalError::Configuration)?;
        broker.validate_root_identities(
            self.core.roots.vault_identity(),
            self.core.roots.writer_data_identity(),
        )?;
        let step = broker
            .archive_step(cutoff, cursor)
            .map_err(LocalError::from);
        self.core.measure_usage(broker);
        // 有在途的事不是故障；别的让旧代失效、安排重扫，和读写入口一样。
        if let Err(error) = &step
            && !matches!(error, LocalError::Read(BrokerError::MutationUnresolved))
        {
            self.core.observe_error(error);
        }
        step
    }
}

impl Core {
    /// 持着写者锁时量一次各处用量，记在内存里；量不出来就记成没有，健康页报未知。
    pub(super) fn measure_usage(&self, broker: &FsVaultBroker) {
        *self.usage.lock().unwrap_or_else(PoisonError::into_inner) = broker.record_usage().ok();
    }
    fn cached_usage(&self) -> Option<RecordUsage> {
        *self.usage.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RecordUsageProbe for CanonicalWorkbench {
    /// 只读内存里最近一次量到的，不碰磁盘、不等写者锁。
    fn record_usage(&self) -> Option<RecordUsage> {
        self.core.cached_usage()
    }
}
