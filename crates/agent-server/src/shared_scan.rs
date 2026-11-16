//! 整读校验的合流（T-211）：并发的内容请求不再一人做一次整读，受理之后才开始的那一次可以共用。
//!
//! 托管副本模式每次内容请求都要完整校验副本（生产 14 MiB 约 2.4 秒），校验又只在一条专用线程上
//! 串行做（`crate::verifier`，BUG-034）。原先 16 路并发就是 16 次整读，最后一个要等约 40 秒，
//! 关停时也排不完。
//!
//! 合流不放宽「每个请求拿到的都是它到达之后重新完整校验过的代次」这条约束，规则只有两条：
//!
//! 1. 一次校验只能给**在它开始之前就已受理**的请求用。请求受理时记下「到这一刻已经开始过几次校验」，
//!    序号比它大的校验，全部磁盘读取都发生在受理之后。
//! 2. 只有**最近完成的那一次成功了**才共用。最近一次失败了，或者根本没有新的，就自己做一次：
//!    失败不外传给别的请求，也绝不退回去用失败之前的旧结果。
//!
//! 于是 N 路同时到达的请求最多做两次整读（在途的一次，加上盖住其余所有人的一次）；
//! 一个接一个到达的请求仍然一人一次，行为和原先一样。

use std::sync::{
    Mutex, MutexGuard, PoisonError,
    atomic::{AtomicU64, Ordering},
};

/// 请求受理时的水位：到那一刻为止已经开始过几次校验。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Admitted(u64);

/// 整读校验的串行闸与合流账。校验在锁内做，所以完成顺序就是序号顺序。
pub(crate) struct SharedScan<T> {
    /// 已经开始过几次校验；受理点只读它，不拿锁。
    started: AtomicU64,
    /// 最近完成的那一次：成功时是（序号, 结果），失败或还没做过是 `None`。
    last: Mutex<Option<(u64, T)>>,
}

impl<T: Clone> SharedScan<T> {
    pub(crate) fn new() -> Self {
        Self {
            started: AtomicU64::new(0),
            last: Mutex::new(None),
        }
    }

    /// 受理点。必须在请求进入排队之前调用。
    pub(crate) fn admit(&self) -> Admitted {
        Admitted(self.started.load(Ordering::SeqCst))
    }

    /// 做一次校验并记账；同一时间只做一次。后台重扫和请求触发的校验都走这里。
    pub(crate) fn run<E>(&self, scan: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        let mut last = self.lock();
        Self::scan_locked(&self.started, &mut last, scan)
    }

    /// 受理之后才开始、并且成功的最近一次校验直接拿来用；否则自己做一次。
    pub(crate) fn covered_or<E>(
        &self,
        admitted: Admitted,
        scan: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        let mut last = self.lock();
        if let Some((ordinal, value)) = last.as_ref()
            && *ordinal > admitted.0
        {
            return Ok(value.clone());
        }
        Self::scan_locked(&self.started, &mut last, scan)
    }

    fn scan_locked<E>(
        started: &AtomicU64,
        last: &mut Option<(u64, T)>,
        scan: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        // 先作废旧结果再开始：校验中途 panic 也不会留下一份看起来还有效的旧账。
        *last = None;
        let ordinal = started.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        let value = scan()?;
        *last = Some((ordinal, value.clone()));
        Ok(value)
    }

    /// 持锁的校验 panic 过也照常用：账在开始时已经作废，不会读到半截状态。
    fn lock(&self) -> MutexGuard<'_, Option<(u64, T)>> {
        self.last.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
#[path = "../tests/verifier_private/shared_scan.rs"]
mod tests;
