//! 可信调度/L2 已批准的单个作业；传输层另核实际 worker peer，本模块不能替代准入。
use std::sync::Arc;
use studiolo_audit_ledger::{
    AuditError, AuditFilter, AuditLedger, AuditPhase, AuditRecord, SqliteAuditLedger,
};
use studiolo_backup::worker::{
    audit::BackupAuditRequest, job::backup_intent, recovery_point::BackupTargetKind, state::JobKey,
};

/// 固定原作业与真实人侧账本，不暴露通用追加或查询。
pub struct BackupAuditPort {
    ledger: Arc<SqliteAuditLedger>,
    key: JobKey,
    target: BackupTargetKind,
}
impl BackupAuditPort {
    /// 只由可信协调器传入已获准的作业；不是从收到的消息中取得批准。
    pub fn new(ledger: Arc<SqliteAuditLedger>, key: JobKey, target: BackupTargetKind) -> Self {
        Self {
            ledger,
            key,
            target,
        }
    }
    /// 已接受的专用连接；实际 peer 必须是批准的独立非 root worker，不能与 App 共用 UID。
    /// 监听器生命周期、配置隔离与并发限额仍由可信启动层负责。
    #[cfg(target_os = "linux")]
    pub fn serve_connection(
        &self,
        stream: std::os::unix::net::UnixStream,
        worker_uid: u32,
        deadline: std::time::Instant,
    ) -> Result<(), studiolo_backup::worker::audit::transport::AuditTransportError> {
        use studiolo_backup::worker::audit::transport::{AuditSocket, AuditTransportError};
        if worker_uid == rustix::process::geteuid().as_raw() {
            return Err(AuditTransportError::Peer);
        }
        let mut socket = AuditSocket::new(stream, worker_uid, deadline)?;
        let bytes = socket.receive()?;
        let response = self
            .dispatch(&bytes)
            .map_err(|_| AuditTransportError::Unavailable)?;
        socket.send(&response)
    }
    /// 单请求单回执；失联重放仍使用同一原键，由权威账本精确幂等。
    pub fn dispatch(&self, bytes: &[u8]) -> Result<Vec<u8>, AuditError> {
        let request = BackupAuditRequest::decode(bytes).map_err(|_| AuditError::Serialization)?;
        if request.key() != &self.key || request.target() != self.target {
            return Err(AuditError::Serialization);
        }
        let _operation = self.ledger.enter_operation()?;
        if request.event().is_some() && request.phase() == AuditPhase::Outcome {
            let expected =
                backup_intent(&self.key, self.target).map_err(|_| AuditError::Serialization)?;
            let original = self
                .query_one(AuditPhase::Intent)?
                .ok_or(AuditError::Unavailable)?;
            if original.event() != &expected {
                return Err(AuditError::CorruptStore);
            }
        }
        let record = match request.event() {
            Some(event) => Some(self.ledger.append(event.clone())?),
            None => self.query_one(request.phase())?,
        };
        request
            .encode_reply(record)
            .map_err(|_| AuditError::CorruptStore)
    }
    fn query_one(&self, phase: AuditPhase) -> Result<Option<AuditRecord>, AuditError> {
        let intent =
            backup_intent(&self.key, self.target).map_err(|_| AuditError::Serialization)?;
        let mut records = self.ledger.query(
            &AuditFilter::default()
                .with_event_id(intent.event_id().clone())
                .with_phase(phase)
                .with_limit(2),
        )?;
        if records.len() > 1 {
            return Err(AuditError::CorruptStore);
        }
        let record = records.pop();
        if let Some(record) = &record {
            record.verify_integrity()?;
        }
        Ok(record)
    }
}
