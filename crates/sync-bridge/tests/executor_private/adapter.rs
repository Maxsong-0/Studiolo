//! 顺序日志包装：持久层映射本身已是生产实现 `crate::sqlite::StateJournal`，
//! 这里只在每次端口调用前记一行顺序日志，供「保全在前、Issued 在中、回执在后」的断言使用。
//! 不复制任何映射逻辑 —— 夹具与生产之间只允许存在一份 BridgeState 适配实现。

use super::ports::{Log, note};
use crate::ActionKey;
use crate::executor::*;
use crate::sqlite::StateJournal;

/// 给生产适配器加一层顺序日志的夹具包装。
pub struct LoggingJournal<'a> {
    /// 真正的生产持久层端口。
    pub inner: StateJournal<'a>,
    /// 顺序日志。
    pub log: Log,
}

impl PlanJournal for LoggingJournal<'_> {
    fn snapshot(&self) -> Result<JournalSnapshot, JournalFault> {
        self.inner.snapshot()
    }
    fn deliver_audit(&mut self, max_records: usize) -> Result<AuditProgress, JournalFault> {
        note(&self.log, "audit.flush".into());
        self.inner.deliver_audit(max_records)
    }
    fn cancel_changed(
        &mut self,
        key: &ActionKey,
        recheck: &ExecutionRecheck,
    ) -> Result<bool, JournalFault> {
        self.inner.cancel_changed(key, recheck)
    }
    fn issue(
        &mut self,
        key: &ActionKey,
        recheck: &ExecutionRecheck,
    ) -> Result<IssuedJob, JournalFault> {
        note(&self.log, "journal.issue".into());
        self.inner.issue(key, recheck)
    }
    fn record_outcome(
        &mut self,
        job: &IssuedJob,
        outcome: &JobOutcome,
    ) -> Result<(), JournalFault> {
        note(&self.log, "journal.record_outcome".into());
        self.inner.record_outcome(job, outcome)
    }
    fn prepare_retirement(&mut self, expected_version: u64) -> Result<(), JournalFault> {
        note(&self.log, "journal.prepare_retirement".into());
        self.inner.prepare_retirement(expected_version)
    }
    fn settle(
        &mut self,
        expected_version: u64,
        sequence: u64,
        common: Option<CommonRecheck>,
    ) -> Result<(), JournalFault> {
        note(
            &self.log,
            format!("journal.settle common={}", common.is_some()),
        );
        self.inner.settle(expected_version, sequence, common)
    }
}
