//! 单个真实作业加一个待扫位；共享 watch 完成状态，不为每次触发创建任务或队列。

use std::{sync::Arc, time::Duration};

use super::{Completion, Control, Core, Failure, WatchHealth};
use crate::LocalError;

impl Core {
    pub fn request(self: &Arc<Self>, reason: &'static str) -> Result<u64, LocalError> {
        self.signal(reason, None, false)
    }

    pub fn watcher_event(self: &Arc<Self>, generation: u64, reason: &'static str, failed: bool) {
        let _ = self.signal(reason, Some(generation), failed);
    }

    fn signal(
        self: &Arc<Self>,
        reason: &'static str,
        generation: Option<u64>,
        failed: bool,
    ) -> Result<u64, LocalError> {
        let mut state = self.control.lock().map_err(|_| LocalError::Unavailable)?;
        if state.closing || generation.is_some_and(|value| value != state.watch_generation) {
            return Err(LocalError::Unavailable);
        }
        if state.requested == u64::MAX || state.epoch == u64::MAX {
            drop(state);
            self.stop();
            return Err(LocalError::Unavailable);
        }
        if failed && state.health == WatchHealth::Starting {
            // 注册自身已失败；当前尝试会返回错误，不将该回调再排成一次相同注册。
            state.epoch += 1;
            state.health = WatchHealth::Failed;
            self.mark_stale(&state, reason);
            self.announce(&state);
            return Ok(state.requested);
        }
        state.requested += 1;
        state.epoch += 1;
        state.pending = true;
        if failed {
            state.health = WatchHealth::Failed;
        }
        self.mark_stale(&state, reason);
        let ticket = state.requested;
        let launch = !state.running;
        state.running = true;
        self.announce(&state);
        drop(state);
        if launch {
            launch_job(self);
        }
        Ok(ticket)
    }

    pub fn stop(self: &Arc<Self>) {
        let mut state = self
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closing {
            return;
        }
        state.closing = true;
        state.pending = false;
        state.health = WatchHealth::Stopped;
        state.result = Some(Failure::Stopped);
        self.mark_stale(&state, "stopping");
        let launch = !state.running;
        state.running = true;
        self.announce(&state);
        drop(state);
        if launch {
            launch_job(self);
        }
    }

    pub async fn wait(&self, ticket: u64) -> Result<(), LocalError> {
        let mut changed = self.completion.subscribe();
        loop {
            let state = *changed.borrow_and_update();
            if state.closed {
                return Err(LocalError::Unavailable);
            }
            if state.finished >= ticket {
                return state
                    .result
                    .map_or(Ok(()), |_| Err(LocalError::Unavailable));
            }
            changed
                .changed()
                .await
                .map_err(|_| LocalError::Unavailable)?;
        }
    }

    pub fn announce(&self, state: &Control) {
        self.completion.send_replace(Completion {
            finished: state.finished,
            running: state.running,
            closed: state.closing,
            result: state.result,
        });
    }

    // 调用方持 control；全局固定 owner → control → view，无反向取得 owner。
    pub fn mark_stale(&self, state: &Control, reason: &'static str) {
        let mut view = self
            .view
            .0
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(freshness) = &mut view.freshness {
            freshness.stale = true;
            freshness.committed_through = None;
            freshness.unresolved = (reason == "unresolved").then_some(true);
            freshness.watcher_healthy = state.health == WatchHealth::Healthy;
            freshness.reason = Some(reason);
        }
    }
}

fn launch_job(core: &Arc<Core>) {
    let running = Arc::clone(core);
    core.executor.spawn_blocking(move || {
        // Arc 留在实际闭包中；请求取消或 Router 丢弃不能让真实作业提前释放 Broker。
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| running.run()));
        if outcome.is_err() {
            running.panic_cleanup();
        }
    });
}

pub(super) fn periodic(core: &Arc<Core>) {
    let weak = Arc::downgrade(core);
    let mut changed = core.completion.subscribe();
    core.executor.spawn(async move {
        let interval = tokio::time::sleep(Duration::from_secs(30));
        tokio::pin!(interval);
        loop {
            tokio::select! {
                _ = &mut interval => {
                    let Some(core) = weak.upgrade() else { return; };
                    if core.request("periodic_check").is_err() { return; }
                    interval.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(30));
                }
                value = changed.changed() => {
                    if value.is_err() || changed.borrow_and_update().closed { return; }
                }
            }
        }
    });
}
