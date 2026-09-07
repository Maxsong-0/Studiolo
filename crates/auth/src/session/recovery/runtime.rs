//! 补记只推进持久 ACK 和失效清理，不向调用者返回权限或原码。

use super::super::SessionError;
use super::{Recovery, pipeline, schema};

impl Recovery {
    /// 补记当前两个原事件并精确重确认被替代的旧证明；只计新增当前 ACK，最多返回 2。
    ///
    /// 本证明交付阶段调用量有界（含全部旧 L3 最多 2076 次），不承诺满表时的固定延迟。
    /// 过期或已撤销操作只有双 ACK 后可释放，绝不自动完成换发或因子替换。
    pub fn flush_audit(&self) -> Result<usize, SessionError> {
        let delivered = self.deliver(None)?;
        self.sessions.run(|db, observed| {
            pipeline::release_invalid(db, observed)?;
            schema::validate(db)?;
            Ok(Ok(()))
        })?;
        Ok(delivered)
    }
}
