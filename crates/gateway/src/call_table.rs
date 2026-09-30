//! 调用幂等表：固定请求身份、有界完成队列，以及从占位时起持有的 RAII 守卫。

#[path = "request_identity.rs"]
mod request_identity;
pub(crate) use request_identity::RequestIdentity;

use crate::{CallId, GatewayError};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Condvar, Mutex},
};
use studiolo_core::AttemptId;

type CallKey = (AttemptId, CallId);

/// 已逐出槽只保留固定宽度请求指纹；请求正文从不进入槽表。
pub(crate) enum CallSlot<Response> {
    InFlight,
    Done {
        identity: RequestIdentity,
        response: Response,
    },
    Evicted(RequestIdentity),
}

/// 同键调用的准入结论；取得占位的结果本身持有释放责任。
pub(crate) enum CallAdmission<'a, Response> {
    Proceed(SlotGuard<'a, Response>),
    /// 与首次请求完全相同的重放：完整重验授权后才能返回首次结果。
    Replay(Response),
    /// 已完成的是不同请求；重新授权后才报告内容冲突。
    Conflict,
    /// 同请求已完成但结果已逐出；重新授权后报告 `CallResultEvicted`。
    Evicted,
}

/// 幂等表最多保留的已完成结果数；按完成顺序逐出结果正文。
/// 历史 call ID 与固定指纹仍保留，防止旧调用被误当成首次执行。
pub const MAX_RETAINED_RESULTS: usize = 4096;

/// 槽与完成队列始终由同一把锁保护；队列中只有仍保有结果的槽。
pub(crate) struct CallState<Response> {
    pub(crate) slots: BTreeMap<CallKey, CallSlot<Response>>,
    pub(crate) completed: VecDeque<CallKey>,
}

/// 检查、占位与完成队列更新都在同一把锁内完成。
pub(crate) struct CallTable<Response> {
    pub(crate) state: Mutex<CallState<Response>>,
    pub(crate) finished: Condvar,
    #[cfg(test)]
    pub(crate) wait_probe: crate::wait_probe::WaitProbe,
}

/// 守卫在占位时建立，覆盖二次授权、审计、执行及 unwind 的全部路径。
pub(crate) struct SlotGuard<'a, Response> {
    table: &'a CallTable<Response>,
    key: Option<CallKey>,
    identity: RequestIdentity,
}

impl<'a, Response> SlotGuard<'a, Response> {
    pub(crate) fn new(
        table: &'a CallTable<Response>,
        key: CallKey,
        identity: RequestIdentity,
    ) -> Self {
        Self {
            table,
            key: Some(key),
            identity,
        }
    }

    pub(crate) fn complete(mut self, response: Response) -> Result<(), GatewayError> {
        let key = self.key.as_ref().ok_or(GatewayError::StateUnavailable)?;
        self.table.complete(key.clone(), self.identity, response)?;
        self.key = None;
        Ok(())
    }
}

impl<Response> Drop for SlotGuard<'_, Response> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            let _ = self.table.abandon(&key);
        }
    }
}

impl<Response> CallTable<Response> {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(CallState {
                slots: BTreeMap::new(),
                completed: VecDeque::new(),
            }),
            finished: Condvar::new(),
            #[cfg(test)]
            wait_probe: crate::wait_probe::WaitProbe::default(),
        }
    }

    fn complete(
        &self,
        key: CallKey,
        identity: RequestIdentity,
        response: Response,
    ) -> Result<(), GatewayError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| GatewayError::StateUnavailable)?;
        if !matches!(state.slots.get(&key), Some(CallSlot::InFlight)) {
            return Err(GatewayError::StateUnavailable);
        }
        if state.completed.len() == MAX_RETAINED_RESULTS
            && let Some(oldest) = state.completed.pop_front()
            && let Some(CallSlot::Done { identity, .. }) = state.slots.remove(&oldest)
        {
            state.slots.insert(oldest, CallSlot::Evicted(identity));
        }
        state.completed.push_back(key.clone());
        state
            .slots
            .insert(key, CallSlot::Done { identity, response });
        drop(state);
        self.finished.notify_all();
        Ok(())
    }

    fn abandon(&self, key: &CallKey) -> Result<(), GatewayError> {
        let result = self
            .state
            .lock()
            .map_err(|_| GatewayError::StateUnavailable)
            .map(|mut state| {
                if matches!(state.slots.get(key), Some(CallSlot::InFlight)) {
                    state.slots.remove(key);
                }
            });
        // 即使锁中毒，也唤醒等待者让它们观察失败，而不是永久停在 Condvar。
        self.finished.notify_all();
        result
    }
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod retention_tests;
