//! 调用幂等表：同键占位、可重放结果与按容量逐出，以及归还占位的 RAII 守卫。

use crate::{CallId, GatewayError};
use std::{
    collections::BTreeMap,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use studiolo_core::AttemptId;

/// 调用幂等槽：`InFlight` 表示同键调用正在执行，`Done` 才携带可重放的结果（成功或已记入
/// 账本的失败终态）；`Evicted` 只保留请求身份，结果已按容量逐出，同请求重放得到结构化拒绝。
pub(crate) enum CallSlot<Request, Response> {
    InFlight,
    Done {
        request: Request,
        response: Response,
        sequence: u64,
    },
    Evicted(Request),
}

/// 同键调用的准入结论。
pub(crate) enum CallAdmission<Response> {
    /// 首次调用，已占位；调用方持有 [`SlotGuard`]，以 `complete` 或丢弃守卫收尾。
    Proceed,
    /// 与首次请求完全相同的重放：完整重验授权后才能返回首次结果。
    Replay(Response),
    /// 已完成的是不同请求；重新授权后才报告内容冲突。
    Conflict,
    /// 同请求已完成但结果已逐出；重新授权后报告 `CallResultEvicted`。
    Evicted,
}

/// 幂等表最多保留的已完成结果数；超出后按完成顺序把最早的结果逐出为 `Evicted`。
/// 结果正文（arguments / output / prompt / text）随逐出释放，不随进程寿命无界驻留。
pub const MAX_RETAINED_RESULTS: usize = 4096;

/// 按 `(attempt, call)` 键串行化同一调用：并发同键请求只执行一次，其余等待首次结果后再判定
/// 重放或冲突。检查与占位在同一把锁内完成，不存在「都通过检查、各执行一次」的窗口。
pub(crate) struct CallTable<Request, Response> {
    pub(crate) slots: Mutex<BTreeMap<(AttemptId, CallId), CallSlot<Request, Response>>>,
    pub(crate) finished: Condvar,
    next_sequence: AtomicU64,
    #[cfg(test)]
    pub(crate) wait_probe: crate::wait_probe::WaitProbe,
}

/// 占位守卫：取得 `Proceed` 的调用无论正常返回、提前返回还是 panic 展开，都归还自己的
/// 占位并唤醒等待者；只有显式 `complete` 才把占位替换为可重放的结果。
pub(crate) struct SlotGuard<'a, Request: PartialEq, Response: Clone> {
    table: &'a CallTable<Request, Response>,
    key: Option<(AttemptId, CallId)>,
}

impl<'a, Request: PartialEq, Response: Clone> SlotGuard<'a, Request, Response> {
    pub(crate) fn new(table: &'a CallTable<Request, Response>, key: (AttemptId, CallId)) -> Self {
        Self {
            table,
            key: Some(key),
        }
    }

    pub(crate) fn complete(
        mut self,
        request: Request,
        response: Response,
    ) -> Result<(), GatewayError> {
        let key = self.key.take().ok_or(GatewayError::StateUnavailable)?;
        self.table.complete(key, request, response)
    }
}

impl<Request: PartialEq, Response: Clone> Drop for SlotGuard<'_, Request, Response> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            // 展开或提前返回时锁不可用也无法再做别的；等待者由 abandon 内的 notify_all 唤醒。
            let _ = self.table.abandon(&key);
        }
    }
}

impl<Request: PartialEq, Response: Clone> CallTable<Request, Response> {
    pub(crate) fn new() -> Self {
        Self {
            slots: Mutex::new(BTreeMap::new()),
            finished: Condvar::new(),
            next_sequence: AtomicU64::new(0),
            #[cfg(test)]
            wait_probe: crate::wait_probe::WaitProbe::default(),
        }
    }

    fn complete(
        &self,
        key: (AttemptId, CallId),
        request: Request,
        response: Response,
    ) -> Result<(), GatewayError> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| GatewayError::StateUnavailable)?;
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        slots.insert(
            key,
            CallSlot::Done {
                request,
                response,
                sequence,
            },
        );
        let retained = slots
            .values()
            .filter(|slot| matches!(slot, CallSlot::Done { .. }))
            .count();
        if retained > MAX_RETAINED_RESULTS {
            // 逐出最早完成的结果；保留请求身份，让「结果已逐出」与「从未调用」可区分。
            let oldest = slots
                .iter()
                .filter_map(|(key, slot)| match slot {
                    CallSlot::Done { sequence, .. } => Some((*sequence, key.clone())),
                    _ => None,
                })
                .min_by_key(|(sequence, _)| *sequence)
                .map(|(_, key)| key);
            if let Some(key) = oldest
                && let Some(CallSlot::Done { request, .. }) = slots.remove(&key)
            {
                slots.insert(key, CallSlot::Evicted(request));
            }
        }
        drop(slots);
        self.finished.notify_all();
        Ok(())
    }

    pub(crate) fn abandon(&self, key: &(AttemptId, CallId)) -> Result<(), GatewayError> {
        self.slots
            .lock()
            .map_err(|_| GatewayError::StateUnavailable)?
            .remove(key);
        self.finished.notify_all();
        Ok(())
    }
}
