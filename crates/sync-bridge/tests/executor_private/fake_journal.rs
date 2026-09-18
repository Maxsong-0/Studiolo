//! 纯内存持久层假实现；用于把整批门槛与审计失败从 SQLite 细节里隔离出来。

use super::ports::Log;
use crate::executor::*;
use crate::*;

/// 可完全预设的持久层替身。
pub struct FakeJournal {
    /// 当前状态版本。
    pub version: u64,
    /// 当前未闭合计划序号。
    pub pending_sequence: Option<u64>,
    /// 当前共同基线版本。
    pub baseline_version: u64,
    /// 已退休序号下界。
    pub retired_through: u64,
    /// 是否已熔断。
    pub fused: bool,
    /// 逐动作状态。
    pub jobs: Vec<JobRecord>,
    /// 仍待确认的审计事件数。
    pub audit_pending: usize,
    /// 审计直接失败时返回的原因。
    pub audit_fault: Option<JournalFault>,
    /// 审计永远推不动。
    pub audit_stalled: bool,
    /// 本次实际 Issued 的原键。
    pub issued: Vec<ActionKey>,
    /// 本次实际保存的终态。
    pub outcomes: Vec<(ActionKey, JobOutcome)>,
    /// settle 的序号与是否带完整共同观察。
    pub settled: Option<(u64, bool)>,
    /// 顺序日志。
    pub log: Log,
}

impl FakeJournal {
    /// 按冻结计划生成一组「Intent 已确认、尚未 Issued」的动作状态。
    pub fn for_plan(plan: &Plan, log: Log) -> Self {
        let sequence = plan.key().sequence;
        let jobs = plan
            .actions()
            .iter()
            .enumerate()
            .map(|(index, action)| JobRecord {
                key: action.key.clone(),
                external_key: format!("fake-bridge-v1-{sequence}-{index}"),
                state: JobState::Reserved,
            })
            .collect();
        Self {
            version: 1,
            pending_sequence: Some(sequence),
            baseline_version: 1,
            retired_through: 0,
            fused: false,
            jobs,
            audit_pending: 0,
            audit_fault: None,
            audit_stalled: false,
            issued: Vec::new(),
            outcomes: Vec::new(),
            settled: None,
            log,
        }
    }
    fn record(&mut self, key: &ActionKey, state: JobState) {
        if let Some(job) = self.jobs.iter_mut().find(|job| &job.key == key) {
            job.state = state;
        }
    }
}

impl PlanJournal for FakeJournal {
    fn snapshot(&self) -> Result<JournalSnapshot, JournalFault> {
        Ok(JournalSnapshot {
            version: self.version,
            pending_sequence: self.pending_sequence,
            baseline_version: self.baseline_version,
            retired_through: self.retired_through,
            fused: self.fused,
            jobs: self.jobs.clone(),
        })
    }
    fn deliver_audit(&mut self, max_records: usize) -> Result<AuditProgress, JournalFault> {
        self.log.borrow_mut().push("audit.flush".into());
        if let Some(fault) = self.audit_fault {
            return Err(fault);
        }
        if self.audit_stalled {
            return Ok(AuditProgress {
                acknowledged: 0,
                remaining: true,
            });
        }
        let acknowledged = self.audit_pending.min(max_records);
        self.audit_pending -= acknowledged;
        Ok(AuditProgress {
            acknowledged,
            remaining: self.audit_pending > 0,
        })
    }
    fn issue(
        &mut self,
        key: &ActionKey,
        _recheck: &ExecutionRecheck,
    ) -> Result<IssuedJob, JournalFault> {
        self.log.borrow_mut().push("journal.issue".into());
        let external_key = self
            .jobs
            .iter()
            .find(|job| &job.key == key)
            .ok_or(JournalFault::NotFound)?
            .external_key
            .clone();
        self.issued.push(key.clone());
        self.record(key, JobState::Issued);
        Ok(IssuedJob {
            key: key.clone(),
            external_key,
            request_digest: [7; 32],
        })
    }
    fn record_outcome(
        &mut self,
        job: &IssuedJob,
        outcome: &JobOutcome,
    ) -> Result<(), JournalFault> {
        self.log.borrow_mut().push("journal.record_outcome".into());
        let state = match outcome {
            JobOutcome::Committed { .. } => JobState::Committed,
            JobOutcome::NoEffect { .. } => JobState::NoEffect,
            JobOutcome::Unknown => return Err(JournalFault::OutcomeMismatch),
        };
        self.outcomes.push((job.key.clone(), outcome.clone()));
        self.record(&job.key, state);
        Ok(())
    }
    fn prepare_retirement(&mut self, _expected_version: u64) -> Result<(), JournalFault> {
        self.log
            .borrow_mut()
            .push("journal.prepare_retirement".into());
        Ok(())
    }
    fn settle(
        &mut self,
        expected_version: u64,
        sequence: u64,
        common: Option<CommonRecheck>,
    ) -> Result<(), JournalFault> {
        if expected_version != self.version {
            return Err(JournalFault::VersionConflict);
        }
        self.settled = Some((sequence, common.is_some()));
        if common.is_some() {
            self.baseline_version += 1;
        }
        self.pending_sequence = None;
        self.version += 1;
        Ok(())
    }
}
