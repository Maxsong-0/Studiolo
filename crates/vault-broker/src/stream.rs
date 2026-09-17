//! 封闭的流式准备/提交协议；准备不授予范围、会话或审计准入。
mod request;
pub(crate) use request::validate_envelope;
mod commit;
mod prepare;
mod replay;
mod restore;
pub(crate) mod stage;
#[cfg(test)]
mod tests;

use crate::{BlobIdentity, ContentAuditEvent, Precondition, VaultPath};
use std::sync::Arc;

/// 调用方提交精确身份，正文只来自显式固定的 BlobSource。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamMutation {
    /// Vault 内的规范逻辑路径。
    pub path: VaultPath,
    /// create-only 或对当前完整旧版本执行 CAS。
    pub precondition: Precondition,
    /// 不输出到日志的原幂等键。
    pub idempotency_key: String,
    /// 精确字节数与原始 SHA-256。
    pub content: BlobIdentity,
}

/// 同实例、同请求的封闭票据；不可克隆或从磁盘/客户端构造。
pub struct PreparedStream {
    instance: Arc<()>,
    reference: crate::recorded::store::RecordingRef,
    operation: crate::mutation::record::Operation,
    current: crate::blob::current::Current,
    stage: crate::blob::file::ContentProof,
    history: crate::blob::install::HistoryPins,
    intent: ContentAuditEvent,
}
impl PreparedStream {
    /// 持久化时已固定的原事件；严格恢复必须实际投递并核对 ACK。
    pub fn intent(&self) -> &ContentAuditEvent {
        &self.intent
    }
    /// 已完整哈希、同步及路径绑定的拟安装 revision。
    pub fn target_revision(&self) -> crate::Revision {
        self.operation.target_revision
    }
}
