//! 写者旧记录打包的触发（BUG-051、T-235）。每一轮都先收口上次没做完的；不做封存的那一轮，回收完暂存之后再看一眼要不要打。
//!
//! 水位是最近一次成功作业准入里的人侧账本 offset 和内容截止时刻（[`SealMark`]）：还没有成功过的封存就不打。用量到一半才打，
//! 平时这里只读一次内存里的用量。每一步之间放开写者锁，也在这时看是否在关停：关停最多等手上这一步。
use super::{BackupProducer, now_millis};
use crate::{LocalError, backup::queue::SealMark};
use std::{
    sync::PoisonError,
    time::{Duration, Instant},
};
use studiolo_vault_broker::{ArchiveCursor, BrokerError};

/// 一轮最多打这么久；没打完的下一轮从停下的地方接着来。
pub(super) const ROUND_BUDGET: Duration = Duration::from_secs(5);
/// 步与步之间让出这么久，排队的保存先走。
const BREATH: Duration = Duration::from_millis(20);

impl BackupProducer {
    /// 上次打包没做完的，现在收口，不看用量、也不用有水位：留着的孤儿段会让封存一直做不成，所以封存的那一轮也先收。
    /// 交回这一轮还能不能往下打。
    pub(super) fn settle_records(&self) -> bool {
        if self.window.closing_within(Duration::ZERO) {
            return false;
        }
        match self.inputs.workbench.archive_settle() {
            Ok(settled) => {
                if settled {
                    self.seal_line("写者旧记录打包：上次没做完的已收口".to_owned());
                }
                true
            }
            Err(error) => {
                self.stopped(&error);
                false
            }
        }
    }

    /// 打一轮。收口没成、在关停都不会走到这里；步与步之间也看关停。
    pub(crate) fn archive_records(&self, seal: Option<SealMark>) {
        let workbench = &self.inputs.workbench;
        let (Some(seal), Some(batch)) = (seal, workbench.archive_due()) else {
            return;
        };
        let Some(now_ms) = self.archive_now() else {
            return;
        };
        let cutoff = seal.cutoff(now_ms);
        let started = Instant::now();
        let mut cursor = self.take_cursor();
        let mut packed = 0;
        while !self.window.closing_within(BREATH) {
            match workbench.archive_step(cutoff, batch, &mut cursor) {
                Ok(step) => {
                    packed += step.packed;
                    if step.done {
                        cursor = ArchiveCursor::default();
                        break;
                    }
                    if started.elapsed() >= self.round_budget() {
                        break;
                    }
                }
                Err(error) => {
                    self.stopped(&error);
                    break;
                }
            }
        }
        self.keep_cursor(cursor);
        if packed > 0 {
            workbench.measure_records();
            self.seal_line(format!(
                "写者旧记录打包：{packed} 条，{} 毫秒",
                started.elapsed().as_millis()
            ));
        }
    }

    /// 上一轮时间到了停在哪、攒了哪几个。攒着的到写的时候会逐个重核，隔了一轮也不会写错。
    fn take_cursor(&self) -> ArchiveCursor {
        std::mem::take(
            &mut *self
                .archive_cursor
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }
    fn keep_cursor(&self, cursor: ArchiveCursor) {
        *self
            .archive_cursor
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = cursor;
    }

    /// 没做完打一行固定类别。拿不到许可、在关停、写者不在、有没收口的写入或没投递完的审计都不是故障，不打行，下一轮再来。
    fn stopped(&self, error: &LocalError) {
        if !matches!(
            error,
            LocalError::Unavailable | LocalError::Read(BrokerError::MutationUnresolved)
        ) {
            self.seal_line(stopped_line(error));
        }
    }

    fn round_budget(&self) -> Duration {
        #[cfg(not(test))]
        {
            ROUND_BUDGET
        }
        #[cfg(test)]
        {
            *self
                .archive_budget
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        }
    }

    fn archive_now(&self) -> Option<i64> {
        #[cfg(test)]
        if let Some(now) = *self
            .archive_clock
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            return Some(now);
        }
        now_millis().ok()
    }
}

/// 固定类别，不含路径和正文。
fn stopped_line(error: &LocalError) -> String {
    let why = match error {
        LocalError::Read(BrokerError::WriterLimitExceeded) => "段数满了",
        LocalError::Read(BrokerError::CorruptWriterState) => "写者记录核不上",
        LocalError::Read(BrokerError::CaptureChanged) => "打包途中记录变了",
        LocalError::Read(BrokerError::Io { .. }) => "读写失败",
        _ => "写者不可用",
    };
    format!("写者旧记录打包没做完：{why}")
}
