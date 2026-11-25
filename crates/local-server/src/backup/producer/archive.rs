//! 写者旧记录打包的触发（BUG-051、T-235）：不做封存的那一轮，回收完暂存之后看一眼。
//!
//! 水位是最近一次成功作业准入里的人侧账本 offset：还没有成功过的封存就不打。用量到一半才打，
//! 平时这里只读一次内存里的用量。每一步之间放开写者锁，也在这时看是否在关停：关停最多等手上这一步。
use super::{BackupProducer, now_millis};
use crate::LocalError;
use std::time::{Duration, Instant};
use studiolo_vault_broker::{ArchiveCursor, ArchiveCutoff, BrokerError};

/// 一轮最多打这么久；没打完的下一轮从头接着来（打过的已经不是散件）。
const ROUND_BUDGET: Duration = Duration::from_secs(5);
/// 步与步之间让出这么久，排队的保存先走。
const BREATH: Duration = Duration::from_millis(20);

impl BackupProducer {
    pub(super) fn archive_records(&self, sealed_offset: Option<u64>) {
        let workbench = &self.inputs.workbench;
        let Some(sealed_offset) = sealed_offset.filter(|_| workbench.archive_due()) else {
            return;
        };
        let Some(now_ms) = self.archive_now() else {
            return;
        };
        let cutoff = ArchiveCutoff {
            sealed_offset,
            now_ms,
        };
        let started = Instant::now();
        let mut cursor = ArchiveCursor::default();
        let mut packed = 0;
        while !self.window.closing_within(BREATH) {
            match workbench.archive_step(cutoff, &mut cursor) {
                Ok(step) => {
                    packed += step.packed;
                    if step.done || started.elapsed() >= ROUND_BUDGET {
                        break;
                    }
                }
                // 有没收口的写入或没投递完的审计：不是故障，下一轮再来。
                Err(LocalError::Read(BrokerError::MutationUnresolved)) => break,
                Err(error) => {
                    self.seal_line(stopped_line(&error));
                    break;
                }
            }
        }
        if packed > 0 {
            self.seal_line(format!(
                "写者旧记录打包：{packed} 条，{} 毫秒",
                started.elapsed().as_millis()
            ));
        }
    }

    fn archive_now(&self) -> Option<i64> {
        #[cfg(test)]
        if let Some(now) = *self
            .archive_clock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
