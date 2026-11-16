//! 不带键的两个队列命令，走同一个固定 socket、同一套路径和对端核对。
//!
//! 「下一个待办」回来的原键只是线索：正文和账本的权限仍然要用带键的准入请求另取。
use super::{endpoint::Endpoint, identity};
use crate::WorkerConfig;
use std::time::{Duration, Instant};
use studiolo_audit_ledger::AuditError;
use studiolo_backup::worker::{
    audit::{AdmissionFact, BackupQueueRequest, QueueAnswer, QueueCommand},
    recovery_point::BackupTargetKind,
    state::JobKey,
};

/// 原生 worker 只接专用的仅追加目标。
const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;

/// 一次轮询用的客户端；socket 的路径证明在构造时固定，生产者重启之后要重新构造。
pub struct WorkerQueueClient<'a> {
    config: &'a WorkerConfig,
    endpoint: Endpoint,
    deadline: Instant,
}
impl<'a> WorkerQueueClient<'a> {
    /// 配置只能来自固定 root 文件；这里不连接。
    pub fn new(config: &'a WorkerConfig, deadline: Instant) -> Result<Self, AuditError> {
        identity(config)?;
        let endpoint = Endpoint::bind(config, deadline)?;
        Ok(Self {
            config,
            endpoint,
            deadline,
        })
    }
    /// 生产者列为待办的原键里最早的一个；没有就是 `None`。
    pub fn next_pending(&self) -> Result<Option<JobKey>, AuditError> {
        match self.exchange(QueueCommand::NextPending)? {
            QueueAnswer::Pending(record) => {
                let fact = AdmissionFact::from_receipt(&record)?;
                if fact.target() != TARGET {
                    return Err(AuditError::CorruptStore);
                }
                Ok(Some(fact.key().clone()))
            }
            QueueAnswer::Idle => Ok(None),
            _ => Err(AuditError::CorruptStore),
        }
    }
    /// 请生产者判断该不该做一次封存；做不做由它按固定配置定。
    pub fn schedule(&self) -> Result<QueueAnswer, AuditError> {
        match self.exchange(QueueCommand::Schedule)? {
            answer @ (QueueAnswer::Started | QueueAnswer::Outstanding | QueueAnswer::NotDue) => {
                Ok(answer)
            }
            _ => Err(AuditError::CorruptStore),
        }
    }
    fn exchange(&self, command: QueueCommand) -> Result<QueueAnswer, AuditError> {
        let request = BackupQueueRequest::new(TARGET, command);
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
