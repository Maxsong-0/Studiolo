//! 由实际生产者连接取得的单作业能力；字段私有，不能从 DTO 或路径直接构造。
use crate::{ProcessError, Result, WorkerAuditClient, WorkerConfig};
use std::time::{Duration, Instant};
use studiolo_backup::worker::{
    audit::AdmissionFact, recovery_point::BackupTargetKind, state::JobKey,
};

/// 固定 root 配置、实际 peer 与原准入回执共同绑定一次执行窗口。
pub struct ApprovedJob<'a> {
    config: &'a WorkerConfig,
    audit: WorkerAuditClient<'a>,
    fact: AdmissionFact,
    deadline: Instant,
}
impl<'a> ApprovedJob<'a> {
    /// 已知原键的接收/续办；不扫描最新目录，也不由客户端决定来源或延长期限。
    /// 专用 HTTPS REST worker 只接已批准的专用目标；这不宣称其不可变保护已实测。
    pub fn connect(config: &'a WorkerConfig, key: JobKey, deadline: Instant) -> Result<Self> {
        let deadline = deadline.min(Instant::now() + Duration::from_secs(7200));
        let audit = WorkerAuditClient::new(
            config,
            key,
            BackupTargetKind::DedicatedAppendOnlyTarget,
            deadline,
        )
        .map_err(|_| ProcessError::Configuration)?;
        let fact = audit.admitted_source().map_err(|_| ProcessError::Source)?;
        if Instant::now() >= deadline {
            return Err(ProcessError::Deadline);
        }
        Ok(Self {
            config,
            audit,
            fact,
            deadline,
        })
    }
    /// 原作业键；排队和续办不得另造 sequence。
    pub fn key(&self) -> &JobKey {
        self.fact.key()
    }
    /// 原目标类别；独立的能力核验仍决定是否显示不可变保护。
    pub fn target(&self) -> BackupTargetKind {
        self.fact.target()
    }
    /// 只读/追加均局限这个原作业，不能取得真实账本。
    pub fn audit(&self) -> &WorkerAuditClient<'a> {
        &self.audit
    }
    pub(crate) fn config(&self) -> &WorkerConfig {
        self.config
    }
    pub(crate) fn fact(&self) -> &AdmissionFact {
        &self.fact
    }
    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }
    pub(crate) fn check_live(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            return Err(ProcessError::Deadline);
        }
        self.fact.check_live().map_err(|_| ProcessError::Deadline)
    }
}
#[cfg(test)]
#[path = "../tests/support/admitted_binding.rs"]
mod tests;
