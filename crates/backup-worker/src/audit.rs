//! 固定部署 socket 的单作业客户端；每次连接及回执前后核原路径与真实生产者身份。
mod endpoint;
use crate::WorkerConfig;
use endpoint::Endpoint;
use std::time::{Duration, Instant};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, OperationPermit,
};
use studiolo_backup::worker::{
    audit::BackupAuditRequest, job::backup_intent, recovery_point::BackupTargetKind, state::JobKey,
};

/// 不持有任何审计库路径或连接；只能通过已批准的独立生产者核对这一个作业。
pub struct WorkerAuditClient<'a> {
    config: &'a WorkerConfig,
    endpoint: Endpoint,
    key: JobKey,
    target: BackupTargetKind,
    deadline: Instant,
}
impl<'a> WorkerAuditClient<'a> {
    /// 配置只能来自固定 root 文件；新客户端不连接、不追加，路径固定失败即拒绝。
    pub fn new(
        config: &'a WorkerConfig,
        key: JobKey,
        target: BackupTargetKind,
        deadline: Instant,
    ) -> Result<Self, AuditError> {
        identity(config)?;
        let endpoint = Endpoint::bind(config, deadline)?;
        Ok(Self {
            config,
            endpoint,
            key,
            target,
            deadline,
        })
    }
    /// 固定原键/阶段的只读续办；不读取或重新上传正文。
    pub fn read_phase(&self, phase: AuditPhase) -> Result<Option<AuditRecord>, AuditError> {
        self.exchange(BackupAuditRequest::read(
            self.key.clone(),
            self.target,
            phase,
        ))
    }
    fn exchange(&self, request: BackupAuditRequest) -> Result<Option<AuditRecord>, AuditError> {
        let deadline = self.deadline.min(Instant::now() + Duration::from_secs(10));
        identity(self.config)?;
        let bytes = request.encode().map_err(|_| AuditError::Serialization)?;
        let mut socket = self.endpoint.connect(deadline)?;
        identity(self.config)?;
        socket.send(&bytes).map_err(|_| AuditError::Unavailable)?;
        let response = socket.receive().map_err(|_| AuditError::Unavailable)?;
        self.endpoint.check(deadline)?;
        identity(self.config)?;
        request
            .decode_reply(&response)
            .map_err(|_| AuditError::CorruptStore)
    }
}
fn identity(config: &WorkerConfig) -> Result<(), AuditError> {
    if rustix::process::getuid() != rustix::process::geteuid()
        || rustix::process::getgid() != rustix::process::getegid()
        || rustix::process::geteuid().as_raw() != config.worker_uid()
        || config.worker_uid() == config.producer_uid()
    {
        return Err(AuditError::UnsafeStorage);
    }
    Ok(())
}
impl AuditLedger for WorkerAuditClient<'_> {
    // 跨进程不伪造“无协调许可”；生产者 dispatch 自己持原账本的完整操作许可。
    fn enter_operation(&self) -> Result<OperationPermit, AuditError> {
        Err(AuditError::Unavailable)
    }
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let request = BackupAuditRequest::append(self.key.clone(), self.target, event)
            .map_err(|_| AuditError::Serialization)?;
        self.exchange(request)?.ok_or(AuditError::CorruptStore)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        let phase = query_phase(filter, &self.key, self.target)?;
        Ok(self.read_phase(phase)?.into_iter().collect())
    }
}
fn query_phase(
    filter: &AuditFilter,
    key: &JobKey,
    target: BackupTargetKind,
) -> Result<AuditPhase, AuditError> {
    let intent = backup_intent(key, target).map_err(|_| AuditError::Serialization)?;
    let phase = filter.phase().ok_or(AuditError::Serialization)?;
    if filter.event_id() != Some(intent.event_id())
        || filter.task_id().is_some()
        || filter.attempt_id().is_some()
        || filter.tool().is_some()
        || filter.offset_exclusive().is_some()
        || filter.limit() != Some(1)
        || filter.action().is_some_and(|a| a != intent.action())
    {
        return Err(AuditError::Serialization);
    }
    Ok(phase)
}
#[cfg(test)]
#[path = "../tests/support/audit_filter.rs"]
mod tests;
