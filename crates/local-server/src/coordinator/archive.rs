//! 写者旧记录打包的入口（BUG-051、T-235）：只由备份后台线程在不封存的那一轮调，不开放给任何 HTTP 入口。
//! 和写入走同一套许可与锁：一步打完才放开，封存等得到它，它也不会夹在封存中间。
use super::{CanonicalWorkbench, Core};
use crate::{LocalError, bounded_lock, health::RecordUsageProbe};
use std::{
    sync::PoisonError,
    time::{Duration, Instant},
};
use studiolo_vault_broker::{
    ArchiveBatch, ArchiveCursor, ArchiveCutoff, ArchiveStep, BrokerError, FsVaultBroker,
    RecordUsage,
};

/// 用量量过之后多久之内不再量：量一次要在写者锁里数四个目录，重扫 30 秒一次、写入之后也会来。
const USAGE_FRESH: Duration = Duration::from_secs(60);

/// 最近一次量到的写者记录用量。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CachedUsage {
    /// 没量过、上一次没量成是没有。
    value: Option<RecordUsage>,
    /// 量成的时刻；打过包就清掉，等着重量。
    measured: Option<Instant>,
}
impl CachedUsage {
    /// 该重量了：没量成过、打过包、或者放久了。
    fn expired(&self, now: Instant) -> bool {
        self.measured
            .is_none_or(|at| now.saturating_duration_since(at) >= USAGE_FRESH)
    }
    #[cfg(test)]
    pub(crate) fn at(value: RecordUsage, measured: Instant) -> Self {
        Self {
            value: Some(value),
            measured: Some(measured),
        }
    }
}

impl CanonicalWorkbench {
    /// 该打包的话按什么批量；只看内存里最近一次的用量，没有就是还不该。
    pub(crate) fn archive_due(&self) -> Option<ArchiveBatch> {
        self.core.cached_usage().value?.archive_batch()
    }

    /// 走一步。拿不到许可或锁、在关停、写者不在都是「不可用」，调用方下一轮再来。
    pub(crate) fn archive_step(
        &self,
        cutoff: ArchiveCutoff,
        batch: ArchiveBatch,
        cursor: &mut ArchiveCursor,
    ) -> Result<ArchiveStep, LocalError> {
        let step = self.with_writer(|broker| broker.archive_step(cutoff, batch, cursor));
        // 打过包手上的用量就不准了；不是每步都量，一轮打完再量（[`Self::measure_records`]）。
        if step.as_ref().is_ok_and(|step| step.packed > 0) {
            self.core.expire_usage();
        }
        step
    }

    /// 上次打包没做完的收口；没有就是假。
    pub(crate) fn archive_settle(&self) -> Result<bool, LocalError> {
        let settled = self.with_writer(|broker| broker.archive_settle());
        if matches!(settled, Ok(true)) {
            self.core.expire_usage();
        }
        settled
    }

    /// 一轮打完量一次，健康页读的就是它；拿不到锁就等下一次重扫。
    pub(crate) fn measure_records(&self) {
        let _ = self.with_writer(|broker| {
            self.core.measure_usage(broker);
            Ok(())
        });
    }

    /// 和写入走同一套许可与锁，核过根再动写者。
    fn with_writer<T>(
        &self,
        work: impl FnOnce(&mut FsVaultBroker) -> Result<T, BrokerError>,
    ) -> Result<T, LocalError> {
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
        let done = work(broker).map_err(LocalError::from);
        // 有在途的事不是故障；别的让旧代失效、安排重扫，和读写入口一样。
        if let Err(error) = &done
            && !matches!(error, LocalError::Read(BrokerError::MutationUnresolved))
        {
            self.core.observe_error(error);
        }
        done
    }
}

impl Core {
    /// 持着写者锁时量一次各处用量，记在内存里；量不出来就记成没有，健康页报未知，下一次重扫再量。
    fn measure_usage(&self, broker: &FsVaultBroker) {
        let value = broker.record_usage().ok();
        *self.usage.lock().unwrap_or_else(PoisonError::into_inner) = CachedUsage {
            value,
            measured: value.map(|_| Instant::now()),
        };
    }
    /// 重扫用：还新就不量，不让每次重扫都在写者锁里数一遍目录。
    pub(super) fn refresh_usage(&self, broker: &FsVaultBroker) {
        if self.cached_usage().expired(Instant::now()) {
            self.measure_usage(broker);
        }
    }
    /// 值照样给健康页看，但下一次重扫一定重量。
    fn expire_usage(&self) {
        self.usage
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .measured = None;
    }
    fn cached_usage(&self) -> CachedUsage {
        *self.usage.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RecordUsageProbe for CanonicalWorkbench {
    /// 只读内存里最近一次量到的，不碰磁盘、不等写者锁。
    fn record_usage(&self) -> Option<RecordUsage> {
        self.core.cached_usage().value
    }
}
