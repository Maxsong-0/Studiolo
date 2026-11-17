//! 没有作业键的三条封闭命令：问「下一个待办」，请求按固定策略安排一次封存，和空闲时问
//! 「该不该抽查仓库、查哪组」。
//!
//! 答复自洽不等于授权：`pending` 只把原准入回执递给 worker，worker 还要拿着其中的原键
//! 再走一遍带键的准入查询；`schedule` 做不做完全由生产者按自己的固定配置判断；
//! `repository_check` 查不查、查哪组也只由生产者按账本定，worker 拿到的是已经落账的原意图。
use super::{AdmissionFact, AuditMessageError, codec};
use crate::worker::{
    recovery_point::BackupTargetKind,
    repository_check::{CheckEvent, read_check_event},
};
use serde::{Deserialize, Serialize};
use studiolo_audit_ledger::AuditPhase;
use studiolo_audit_ledger::AuditRecord;
type Result<T> = std::result::Result<T, AuditMessageError>;

/// 队列命令；和带键的审计消息字段不同，两种消息互相解不出来。
pub struct BackupQueueRequest(Fields);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    schema: u32,
    target: BackupTargetKind,
    queue: QueueCommand,
}
/// 三条命令都不带参数：没有路径、序号、组号或时间可以由 worker 指定。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueCommand {
    /// 取已准入、还没有结果的原键里最早的一个。
    NextPending,
    /// 请求安排一次封存；生产者先回收过期暂存，再按固定策略决定做不做。
    Schedule,
    /// 没有待办时问该不该抽查仓库、查哪组；生产者到期就先把原意图落账再答。
    RepositoryCheck,
}
/// 生产者的固定答复。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueAnswer {
    /// 有待办：原准入回执。
    Pending(Box<AuditRecord>),
    /// 没有待办。
    Idle,
    /// 已开始一次封存；做成之后会出现在「下一个待办」里。
    Started,
    /// 还有没做完的作业或封存，这次不做。
    Outstanding,
    /// 距上次不够间隔，这次不做。
    NotDue,
    /// 派下一次抽查：已经落账的原意图回执。
    CheckAssigned(Box<AuditRecord>),
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Pending,
    Idle,
    Started,
    Outstanding,
    NotDue,
    CheckAssigned,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    schema: u32,
    request_sha256: String,
    answer: Kind,
    record: Option<AuditRecord>,
}
impl BackupQueueRequest {
    /// 目标类别由 worker 的固定配置给出；生产者必须和自己的配置核对。
    pub fn new(target: BackupTargetKind, command: QueueCommand) -> Self {
        Self(Fields {
            schema: 1,
            target,
            queue: command,
        })
    }
    /// 有界规范消息。
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.0.schema != 1 {
            return Err(AuditMessageError::Invalid);
        }
        codec::encode(&self.0)
    }
    /// 只接受同一规范编码；带键的审计消息、未知字段和尾随数据都拒绝。
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        codec::size(bytes)?;
        let request = Self(serde_json::from_slice(bytes).map_err(|_| AuditMessageError::Invalid)?);
        if request.encode()? != bytes {
            return Err(AuditMessageError::Invalid);
        }
        Ok(request)
    }
    /// 请求声明的目标类别。
    pub fn target(&self) -> BackupTargetKind {
        self.0.target
    }
    /// 请求的命令。
    pub fn command(&self) -> QueueCommand {
        self.0.queue
    }
    /// 生产者编码答复；答复种类必须属于这条命令，`pending` 必须带同目标的原准入回执。
    pub fn encode_reply(&self, answer: &QueueAnswer) -> Result<Vec<u8>> {
        self.check(answer)?;
        let (kind, record) = match answer {
            QueueAnswer::Pending(record) => (Kind::Pending, Some(record.as_ref().clone())),
            QueueAnswer::Idle => (Kind::Idle, None),
            QueueAnswer::Started => (Kind::Started, None),
            QueueAnswer::Outstanding => (Kind::Outstanding, None),
            QueueAnswer::NotDue => (Kind::NotDue, None),
            QueueAnswer::CheckAssigned(record) => {
                (Kind::CheckAssigned, Some(record.as_ref().clone()))
            }
        };
        codec::encode(&Reply {
            schema: 1,
            request_sha256: codec::digest(&self.encode()?),
            answer: kind,
            record,
        })
    }
    /// worker 解码答复：核请求摘要、规范编码、答复种类和回执形状。
    pub fn decode_reply(&self, bytes: &[u8]) -> Result<QueueAnswer> {
        codec::size(bytes)?;
        let reply: Reply = serde_json::from_slice(bytes).map_err(|_| AuditMessageError::Invalid)?;
        if reply.schema != 1
            || reply.request_sha256 != codec::digest(&self.encode()?)
            || codec::encode(&reply)? != bytes
        {
            return Err(AuditMessageError::Invalid);
        }
        let answer = match (reply.answer, reply.record) {
            (Kind::Pending, Some(record)) => QueueAnswer::Pending(Box::new(record)),
            (Kind::Idle, None) => QueueAnswer::Idle,
            (Kind::Started, None) => QueueAnswer::Started,
            (Kind::Outstanding, None) => QueueAnswer::Outstanding,
            (Kind::NotDue, None) => QueueAnswer::NotDue,
            (Kind::CheckAssigned, Some(record)) => QueueAnswer::CheckAssigned(Box::new(record)),
            _ => return Err(AuditMessageError::Invalid),
        };
        self.check(&answer)?;
        Ok(answer)
    }
    fn check(&self, answer: &QueueAnswer) -> Result<()> {
        let fits = match (self.0.queue, answer) {
            (QueueCommand::NextPending, QueueAnswer::Pending(record)) => {
                record
                    .verify_integrity()
                    .map_err(|_| AuditMessageError::Invalid)?;
                let fact =
                    AdmissionFact::from_receipt(record).map_err(|_| AuditMessageError::Invalid)?;
                fact.target() == self.0.target
            }
            (QueueCommand::NextPending, QueueAnswer::Idle) => true,
            (
                QueueCommand::Schedule,
                QueueAnswer::Started | QueueAnswer::Outstanding | QueueAnswer::NotDue,
            ) => true,
            (QueueCommand::RepositoryCheck, QueueAnswer::CheckAssigned(record)) => {
                record
                    .verify_integrity()
                    .map_err(|_| AuditMessageError::Invalid)?;
                record.event().phase() == AuditPhase::Intent
                    && matches!(
                        read_check_event(record.event()),
                        Some(CheckEvent::Intent { target, .. }) if target == self.0.target
                    )
            }
            (QueueCommand::RepositoryCheck, QueueAnswer::Outstanding | QueueAnswer::NotDue) => true,
            _ => false,
        };
        if fits {
            Ok(())
        } else {
            Err(AuditMessageError::Invalid)
        }
    }
}
