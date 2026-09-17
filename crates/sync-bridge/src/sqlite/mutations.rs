//! 小事务应用与回滚；Action 只备份一行，SQL 外不执行动作。
use super::{
    BridgeError, BridgeState, cache, database,
    model::*,
    opening::{checkpoint, data_version},
};
use rusqlite::TransactionBehavior;
impl BridgeState {
    pub(super) fn edit<T>(
        &mut self,
        edit: Edit,
        expected_version: Option<u64>,
        f: impl FnOnce(&mut State) -> Result<T>,
    ) -> Result<T> {
        self.edit_guarded(edit, expected_version, || Ok(()), f)
    }
    pub(super) fn edit_guarded<T>(
        &mut self,
        edit: Edit,
        expected_version: Option<u64>,
        guard: impl Fn() -> Result<()>,
        f: impl FnOnce(&mut State) -> Result<T>,
    ) -> Result<T> {
        self.read()?;
        // 新事务之前清空旧 WAL；读者阻止截断时拒绝新增写，不能叠加最坏帧预算。
        checkpoint(&self.connection)?;
        self.storage.check()?;
        if expected_version.is_some_and(|v| v != self.cache.version) {
            return Err(BridgeError::VersionConflict);
        }
        let previous = self.cache.version;
        let next = previous
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(BridgeError::Overflow)?;
        let undo = cache::Undo::capture(&self.cache, edit)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database::error)?;
        if data_version(&tx)? != self.data_version || database::revision(&tx)? != previous {
            return Err(BridgeError::VersionConflict);
        }
        #[cfg(test)]
        if let Some(hook) = self.before_guard.take() {
            hook();
        }
        let changed = (|| {
            guard()?;
            let result = f(&mut self.cache)?;
            undo.refresh(&mut self.cache)?;
            self.cache.version = next;
            cache::validate(&self.cache, edit)?;
            database::persist(&tx, &self.cache, edit, previous)?;
            self.storage.check()?;
            guard()?;
            Ok(result)
        })();
        let result = match changed {
            Ok(v) => v,
            Err(e) => {
                undo.restore(&mut self.cache);
                self.cache.version = previous;
                return Err(e);
            }
        };
        if tx.commit().is_err() {
            self.poisoned.set(true);
            return Err(BridgeError::Unavailable);
        }
        // 提交后 checkpoint/身份失败也不能说“未发生”；当前实例封闭，重开核对原键。
        if self
            .storage
            .check()
            .and_then(|_| checkpoint(&self.connection))
            .and_then(|_| self.storage.check())
            .is_err()
        {
            self.poisoned.set(true);
            return Err(BridgeError::Unavailable);
        }
        #[cfg(test)]
        match edit {
            Edit::Full => self.stats.full_writes += 1,
            Edit::Action { .. } => self.stats.action_writes += 1,
            _ => {}
        }
        #[cfg(test)]
        if let Some(hook) = self.after_commit.take() {
            hook();
        }
        Ok(result)
    }
}
