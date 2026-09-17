//! 只在显式启用时固定预算；HTTP 单次请求不能增加或重置保留承诺。
use crate::{BrokerError, blob::OBJECT_LIMIT};
use serde::{Deserialize, Serialize};

/// 固定部署审批的文件操作预算；没有默认配置就不能启用。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileOperationLimits {
    /// 本次部署允许的新删除/恢复对象上限，至多 256 MiB。
    pub effective_object_limit: u64,
    /// 候选、已提交和未知墓碑的合计上限，至多 4096。
    pub max_tombstones: u32,
    /// L0 与隔离对象均按最坏逻辑字节计费，不依赖去重或稀疏节省。
    pub retained_bytes_limit: u64,
    /// root/data 各自必须留下的实际可用空间；无法读取空间信息时拒绝。
    pub minimum_free_bytes: u64,
}
impl FileOperationLimits {
    pub(crate) fn validate(self) -> Result<(), BrokerError> {
        let maximum = (OBJECT_LIMIT * 5 + 4 * REQUEST_LIMIT as u64) * RECORD_COUNT as u64
            + crate::blob::STAGE_BYTES
            + RECORD_BYTES as u64
            + TOMBSTONE_BYTES as u64 * 3;
        if self.effective_object_limit == 0
            || self.effective_object_limit > OBJECT_LIMIT
            || !(1..=4096).contains(&self.max_tombstones)
            || self.retained_bytes_limit == 0
            || self.retained_bytes_limit > maximum
            || self.minimum_free_bytes == 0
        {
            return Err(BrokerError::WriterLimitExceeded);
        }
        Ok(())
    }
}

pub(crate) const REQUEST_LIMIT: usize = 64 * 1024;
pub(crate) const RECORD_COUNT: usize = 4096;
pub(crate) const RECORD_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const TOMBSTONE_COUNT: usize = 4096;
pub(crate) const TOMBSTONE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MINIMUM_RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;
