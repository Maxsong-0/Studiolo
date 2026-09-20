//! 只有显式开启升级主日志；旧读取/旧能力初始化不会静默授予 Rename。
use crate::{BrokerError, FsVaultBroker, mutation};
impl FsVaultBroker {
    /// 只读已持久化的 Rename 标志；校验当前主日志和存储绑定，不初始化或确认任何操作。
    /// 未决操作期间仍可查询，true 不表示当前可写、审计完成或人侧 scope 已批准。
    pub fn file_rename_enabled(&self) -> Result<bool, BrokerError> {
        Ok(mutation::load(self)?.rename_enabled)
    }

    /// 显式开启单文件复合 Rename；调用者须先装配完整审计及双路径维护适配。
    /// 不创建存储根，不改变旧请求字节、序号或提交水位。
    pub fn enable_file_rename(&mut self) -> Result<(), BrokerError> {
        crate::maintenance::ensure_idle(self)?;
        super::store::required(self)?;
        let mut state = mutation::load(self)?;
        if self.write_status()?.unresolved || self.content_audit_status()?.restore_unconfirmed {
            return Err(BrokerError::MutationUnresolved);
        }
        if state.rename_enabled {
            return Ok(());
        }
        state.rename_enabled = true;
        state.format_version = state.format_version.max(6);
        mutation::fault(self, 84)?;
        mutation::save(self, &state)?;
        mutation::fault(self, 85)
    }
}
