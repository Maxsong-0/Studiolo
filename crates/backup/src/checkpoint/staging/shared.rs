//! 专用读取组来自受信任部署配置，不来自备份请求或封存里的自报身份。
use super::{Result, StagingError};

/// 生产者与读取组的部署身份；故意不实现 Deserialize，不能直接接 HTTP 输入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReaderIdentity {
    pub(super) producer_uid: u32,
    pub(super) reader_gid: u32,
}
impl ReaderIdentity {
    /// 只在读取固定、受保护的部署配置时构造；拒绝 root 和无效身份。
    pub fn from_config(producer_uid: u32, reader_gid: u32) -> Result<Self> {
        if matches!(producer_uid, 0 | u32::MAX) || matches!(reader_gid, 0 | u32::MAX) {
            return Err(StagingError::UnsafeRoot);
        }
        Ok(Self {
            producer_uid,
            reader_gid,
        })
    }
    pub(super) fn producer(reader_gid: u32) -> Result<Self> {
        Self::from_config(rustix::process::geteuid().as_raw(), reader_gid)
    }
    pub(super) fn require_worker(self) -> Result<()> {
        let worker = rustix::process::geteuid().as_raw();
        if worker == 0 || worker == self.producer_uid {
            return Err(StagingError::UnsafeRoot);
        }
        // 权限读取再由内核实际执行；相同 UID 或 root 不能冒充凭据隔离。
        Ok(())
    }
}
