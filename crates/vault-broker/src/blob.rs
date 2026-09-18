//! 有界附件基础层：源句柄固定、分块哈希、独立于服务协商的本地保全上限。
mod cleanup;
pub use cleanup::BlobPruneReport;
#[cfg(test)]
mod serial_gate;
#[cfg(test)]
pub(crate) use serial_gate::SerialGate;
/// 大文件 IO 用例的串行闸门；只排他，持有者 panic 后由下一位接管，不毒化级联。
#[cfg(test)]
pub(crate) static LARGE_IO: SerialGate = SerialGate::new();
pub(crate) mod current;
mod digest;
pub(crate) mod file;
pub(crate) mod install;
pub(crate) mod observed;
mod paths;
pub(crate) mod quota;
mod source;
pub(crate) mod store;
use crate::{BrokerError, writer::disk};
use serde::{Deserialize, Serialize};
pub use source::BlobSource;

pub(crate) const OBJECT_LIMIT: u64 = 256 * 1024 * 1024;
pub(crate) const STAGE_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const STAGE_COUNT: usize = 64;
pub(crate) const CHUNK_BYTES: usize = 64 * 1024;

/// 已核对的正文身份；摘要是原始字节的 SHA-256，不包含路径或 Vault 身份。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobIdentity {
    /// 精确逻辑字节数，稀疏区域也计入。
    pub bytes: u64,
    /// 64 个小写十六进制字符的 SHA-256。
    pub sha256: String,
}
impl BlobIdentity {
    pub(crate) fn validate(&self) -> Result<(), BrokerError> {
        if self.bytes > OBJECT_LIMIT || !disk::hex(&self.sha256) {
            Err(BrokerError::WriterLimitExceeded)
        } else {
            Ok(())
        }
    }
}

/// 本次新输入的显式有效上限；不代表已验证 Obsidian 服务的产品额度。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamLimits {
    /// 服务协商、配置策略与内部硬上限取最小值，单位为精确字节。
    pub effective_limit: u64,
}
impl StreamLimits {
    /// 显式上限必须为 1..=256 MiB；未知协商值不能用此构造器伪装为已知额度。
    pub fn new(effective_limit: u64) -> Result<Self, BrokerError> {
        let value = Self { effective_limit };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn validate(self) -> Result<(), BrokerError> {
        if self.effective_limit == 0 || self.effective_limit > OBJECT_LIMIT {
            Err(BrokerError::WriterLimitExceeded)
        } else {
            Ok(())
        }
    }
}
