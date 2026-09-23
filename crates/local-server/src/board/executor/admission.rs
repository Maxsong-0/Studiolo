//! 进程创建、冻结与关停由同一道准入屏障排定先后。
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Default)]
pub(super) struct Admission {
    gate: Mutex<()>,
    epoch: AtomicU64,
    closed: AtomicBool,
}
impl Admission {
    pub fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.invalidate();
    }
    pub fn lock(&self) -> Result<MutexGuard<'_, ()>, String> {
        self.gate.lock().map_err(|_| "Runner准入状态不可用".into())
    }
    pub fn start(&self, frozen: &AtomicBool) -> Result<MutexGuard<'_, ()>, String> {
        let lock = self.lock()?;
        if self.closed.load(Ordering::SeqCst) || frozen.load(Ordering::SeqCst) {
            return Err("Runner已冻结或关闭".into());
        }
        Ok(lock)
    }
    pub fn probe_epoch(&self) -> Result<u64, String> {
        let _lock = self.lock()?;
        if self.closed.load(Ordering::SeqCst) {
            return Err("Runner已关闭".into());
        }
        Ok(self.epoch.load(Ordering::SeqCst))
    }
    pub fn unfreeze(
        &self,
        epoch: u64,
        commit: impl FnOnce() -> Result<(), String>,
        rollback: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let _lock = self.lock()?;
        let valid =
            || !self.closed.load(Ordering::SeqCst) && self.epoch.load(Ordering::SeqCst) == epoch;
        if !valid() {
            return Err("解冻期间发生新的冻结或关闭".into());
        }
        commit()?;
        // 紧急冻结可能不等这把锁，就作废尚未完成的启动。
        if !valid() {
            rollback()?;
            return Err("解冻被新的冻结或关闭撤销".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "admission_tests.rs"]
mod tests;
