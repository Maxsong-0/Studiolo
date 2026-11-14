//! 独立 worker 的封闭审计消息；字段与回执自洽不等于 peer 鉴权或作业获准。
mod codec;
#[cfg(target_os = "linux")]
pub mod transport;
mod validation;
use super::{recovery_point::BackupTargetKind, state::JobKey};
pub use codec::{AUDIT_MESSAGE_MAX_BYTES, AuditMessageError};
use serde::{Deserialize, Serialize};
use studiolo_audit_ledger::{AuditEvent, AuditPhase, AuditRecord};
type Result<T> = std::result::Result<T, AuditMessageError>;

/// 只能追加固定形状的本作业备份事件，或读取本作业某一阶段的原回执。
pub struct BackupAuditRequest(Fields);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    schema: u32,
    key: JobKey,
    target: BackupTargetKind,
    command: Command,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Append { event: AuditEvent },
    Read { phase: AuditPhase },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    schema: u32,
    request_sha256: String,
    record: Option<AuditRecord>,
}
impl BackupAuditRequest {
    /// 客户端在传输前校验事件：不能从通用 AuditLedger 端口偷渡其他动作、正文或主体。
    pub fn append(key: JobKey, target: BackupTargetKind, event: AuditEvent) -> Result<Self> {
        let request = Self(Fields {
            schema: 1,
            key,
            target,
            command: Command::Append { event },
        });
        request.validate()?;
        Ok(request)
    }
    /// 只读固定 JobKey/阶段；没有可扩展过滤器或全账本查询。
    pub fn read(key: JobKey, target: BackupTargetKind, phase: AuditPhase) -> Self {
        Self(Fields {
            schema: 1,
            key,
            target,
            command: Command::Read { phase },
        })
    }
    /// 有界规范消息；反序列化私有字段不对调用方开放。
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        codec::encode(&self.0)
    }
    /// 只接受同一规范编码，拒绝未知/重复字段、尾随数据和非规范替代形状。
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        codec::size(bytes)?;
        let request = Self(serde_json::from_slice(bytes).map_err(|_| AuditMessageError::Invalid)?);
        if request.encode()? != bytes {
            return Err(AuditMessageError::Invalid);
        }
        Ok(request)
    }
    /// 消息声明的键不是授权；服务端必须与已经批准的原作业核对。
    pub fn key(&self) -> &JobKey {
        &self.0.key
    }
    /// 服务端必须匹配实际配置的目标类别，不能相信客户端自行升级不可变能力。
    pub fn target(&self) -> BackupTargetKind {
        self.0.target
    }
    /// 要处理的固定阶段。
    pub fn phase(&self) -> AuditPhase {
        match &self.0.command {
            Command::Append { event } => event.phase(),
            Command::Read { phase } => *phase,
        }
    }
    /// 只有 Append 包含已验证的固定事件；Read 不获得追加能力。
    pub fn event(&self) -> Option<&AuditEvent> {
        match &self.0.command {
            Command::Append { event } => Some(event),
            Command::Read { .. } => None,
        }
    }
    /// 服务端只返回本请求的完整原回执；无结果只允许用于 Read。
    pub fn encode_reply(&self, record: Option<AuditRecord>) -> Result<Vec<u8>> {
        self.check_record(record.as_ref())?;
        codec::encode(&Reply {
            schema: 1,
            request_sha256: codec::digest(&self.encode()?),
            record,
        })
    }
    /// 核请求摘要、固定原事件和完整回执哈希；传输层仍须提供真实 peer 身份。
    pub fn decode_reply(&self, bytes: &[u8]) -> Result<Option<AuditRecord>> {
        codec::size(bytes)?;
        let reply: Reply = serde_json::from_slice(bytes).map_err(|_| AuditMessageError::Invalid)?;
        if reply.schema != 1
            || reply.request_sha256 != codec::digest(&self.encode()?)
            || codec::encode(&reply)? != bytes
        {
            return Err(AuditMessageError::Invalid);
        }
        self.check_record(reply.record.as_ref())?;
        Ok(reply.record)
    }
    fn validate(&self) -> Result<()> {
        if self.0.schema != 1 {
            return Err(AuditMessageError::Invalid);
        }
        codec::encode(&self.0)?;
        if let Some(event) = self.event() {
            validation::event(event, self.key(), self.target())?;
        }
        Ok(())
    }
    fn check_record(&self, record: Option<&AuditRecord>) -> Result<()> {
        self.validate()?;
        let Some(record) = record else {
            return if self.event().is_none() {
                Ok(())
            } else {
                Err(AuditMessageError::Invalid)
            };
        };
        codec::encode(record)?;
        record
            .verify_integrity()
            .map_err(|_| AuditMessageError::Invalid)?;
        validation::event(record.event(), self.key(), self.target())?;
        if record.event().phase() != self.phase()
            || self.event().is_some_and(|e| e != record.event())
        {
            return Err(AuditMessageError::Invalid);
        }
        Ok(())
    }
}
