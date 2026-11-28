//! 消息编码过程即限额；不先分配无限 Vec 再检查长度。
use serde::Serialize;
use std::io::Write;
/// 请求与回执各最多 16KiB，协议不允许正文。
pub const AUDIT_MESSAGE_MAX_BYTES: usize = 16 * 1024;
/// 固定错误类别，不回显任何消息字段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditMessageError {
    /// 事件形状、作业/阶段、规范编码或回执不符。
    Invalid,
    /// 消息超过固定资源预算。
    TooLarge,
}
impl std::fmt::Display for AuditMessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "备份审计消息无效：{self:?}")
    }
}
impl std::error::Error for AuditMessageError {}
pub(super) fn size(bytes: &[u8]) -> Result<(), AuditMessageError> {
    if bytes.is_empty() || bytes.len() > AUDIT_MESSAGE_MAX_BYTES {
        Err(AuditMessageError::TooLarge)
    } else {
        Ok(())
    }
}
pub(super) fn encode(value: &impl Serialize) -> Result<Vec<u8>, AuditMessageError> {
    let mut output = Limited(Vec::new());
    serde_json::to_writer(&mut output, value).map_err(|_| AuditMessageError::TooLarge)?;
    Ok(output.0)
}
struct Limited(Vec<u8>);
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > AUDIT_MESSAGE_MAX_BYTES - self.0.len() {
            return Err(std::io::Error::other("审计消息超过预算"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn digest(bytes: &[u8]) -> String {
    studiolo_core::sha256_hex(bytes)
}
