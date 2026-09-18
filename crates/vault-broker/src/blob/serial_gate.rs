//! 大文件 IO 串行闸门：只提供互斥排队，不保护任何共享可变状态。
//!
//! `std::sync::Mutex` 的毒化语义是为「守卫内破坏了不变量」准备的；本闸门的守卫是 `()`，
//! 没有不变量可破坏，毒化在此没有保护价值，反而把「一个用例失败」放大成「此后每个排队
//! 用例都 `PoisonError`」，真实根因被级联淹没。故本类型对外只暴露「可接管」的获取方式，
//! 调用方拿不到毒化状态，也就无从误用。

use std::sync::{Mutex, MutexGuard, PoisonError};

/// 只排他、持有者 panic 后可安全接管的串行闸门。
pub(crate) struct SerialGate(Mutex<()>);

impl SerialGate {
    /// 构造闸门；可直接用于 `static`，不需要惰性初始化。
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(()))
    }

    /// 排队进入闸门；上一位持有者 panic 留下的毒化按「可接管」处理。
    pub(crate) fn enter(&self) -> MutexGuard<'_, ()> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
