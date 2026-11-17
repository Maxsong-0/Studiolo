//! worker 上报一次仓库抽查的结果：检查键、三类结果和它检查的仓库摘要。
//!
//! 和带键的审计消息、队列命令字段都不同，三种消息互相解不出来。消息只能导出唯一一条结果事件；
//! 生产者还要核账本里有同一检查键的原意图，才把它追加进去。没有正文、路径或 restic 原始输出。
use super::{AuditMessageError, codec};
use crate::worker::{
    recovery_point::BackupTargetKind,
    repository_check::{
        CheckKey, CheckOutcome, CheckRecorder, IncompleteReason, RepositoryDigest, check_outcome,
    },
};
use serde::{Deserialize, Serialize};
use studiolo_audit_ledger::{AuditEvent, AuditRecord};
type Result<T> = std::result::Result<T, AuditMessageError>;

/// 一次抽查的结果上报。
pub struct RepositoryCheckReport(Fields);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    schema: u32,
    target: BackupTargetKind,
    check: CheckKey,
    outcome: CheckOutcome,
    repository: RepositoryDigest,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    schema: u32,
    request_sha256: String,
    record: AuditRecord,
}
impl RepositoryCheckReport {
    /// worker 组装上报；「中途没了」只能由生产者补记，worker 不能报。
    pub fn new(
        target: BackupTargetKind,
        check: CheckKey,
        outcome: CheckOutcome,
        repository: RepositoryDigest,
    ) -> Result<Self> {
        let report = Self(Fields {
            schema: 1,
            target,
            check,
            outcome,
            repository,
        });
        report.validate()?;
        Ok(report)
    }
    /// 有界规范消息。
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        codec::encode(&self.0)
    }
    /// 只接受同一规范编码；别的两种消息、未知字段和尾随数据都拒绝。
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        codec::size(bytes)?;
        let report = Self(serde_json::from_slice(bytes).map_err(|_| AuditMessageError::Invalid)?);
        if report.encode()? != bytes {
            return Err(AuditMessageError::Invalid);
        }
        Ok(report)
    }
    /// 声明的目标类别；生产者要和自己的固定配置核对。
    pub fn target(&self) -> BackupTargetKind {
        self.0.target
    }
    /// 检查键；生产者要在账本里找到同一键的原意图。
    pub fn key(&self) -> &CheckKey {
        &self.0.check
    }
    /// 三类结果。
    pub fn outcome(&self) -> &CheckOutcome {
        &self.0.outcome
    }
    /// 检查的仓库摘要。
    pub fn repository(&self) -> &RepositoryDigest {
        &self.0.repository
    }
    /// 这份上报唯一能落账的结果事件。
    pub fn event(&self) -> Result<AuditEvent> {
        check_outcome(
            &self.0.check,
            &self.0.outcome,
            &CheckRecorder::Worker(self.0.repository.clone()),
        )
        .map_err(|_| AuditMessageError::Invalid)
    }
    /// 生产者回原结果回执：必须就是这份上报导出的那条事件。
    pub fn encode_reply(&self, record: &AuditRecord) -> Result<Vec<u8>> {
        self.check_record(record)?;
        codec::encode(&Reply {
            schema: 1,
            request_sha256: codec::digest(&self.encode()?),
            record: record.clone(),
        })
    }
    /// worker 核请求摘要、规范编码、回执哈希和事件本身。
    pub fn decode_reply(&self, bytes: &[u8]) -> Result<AuditRecord> {
        codec::size(bytes)?;
        let reply: Reply = serde_json::from_slice(bytes).map_err(|_| AuditMessageError::Invalid)?;
        if reply.schema != 1
            || reply.request_sha256 != codec::digest(&self.encode()?)
            || codec::encode(&reply)? != bytes
        {
            return Err(AuditMessageError::Invalid);
        }
        self.check_record(&reply.record)?;
        Ok(reply.record)
    }
    fn validate(&self) -> Result<()> {
        if self.0.schema != 1 || self.0.outcome.reason() == Some(IncompleteReason::Abandoned) {
            return Err(AuditMessageError::Invalid);
        }
        self.event().map(drop)
    }
    fn check_record(&self, record: &AuditRecord) -> Result<()> {
        codec::encode(record)?;
        record
            .verify_integrity()
            .map_err(|_| AuditMessageError::Invalid)?;
        if record.event() != &self.event()? {
            return Err(AuditMessageError::Invalid);
        }
        Ok(())
    }
}
