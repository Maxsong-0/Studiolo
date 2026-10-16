//! 关停（T-184，vault BUG-033）：信号一到就不再接新会话和新请求，等已进门的 MCP POST 做完再关全部会话；
//! 会话一关，名下的 SSE 流随之结束，axum 的优雅关停才等得到连接收尾。客户端开着的 GET 流不会自己结束，
//! 不主动关的话进程会一直等，直到被 systemd 强杀。整个过程有上限，赶在单元的 `TimeoutStopSec=20` 之前退出。

use std::{
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use tokio::sync::Notify;

/// 已进门的 MCP 请求最多等这么久；到点仍没做完就强制关会话，响应随之截断。
pub(crate) const DRAIN_LIMIT: Duration = Duration::from_secs(10);
/// 从收到信号到 `serve::run` 返回的总上限，含 `DRAIN_LIMIT`、连接收尾和后台重扫收尾。
pub(crate) const STOP_LIMIT: Duration = Duration::from_secs(15);
/// `serve::run` 返回之后，运行时再给阻塞线程的收尾时间，由 `main` 使用。重扫与副本校验不在其列：
/// T-185 起它们在专用校验线程上（`crate::verifier`，BUG-034），收尾见那里。
/// `STOP_LIMIT + RUNTIME_LIMIT`（17 秒）必须小于 agent 各单元的 `TimeoutStopSec=20`
/// （`deploy/agent/` 的两个单元模板和三个 systemd-run 脚本）。
pub const RUNTIME_LIMIT: Duration = Duration::from_secs(2);
/// 关停开始后拒绝新请求时记进审计的原因；两个外壳共用，对外回 503。
pub(crate) const SERVER_STOPPING: &str = "server_stopping";

#[derive(Default)]
struct Phase {
    stopping: bool,
    in_flight: usize,
}

/// 两个外壳共用的关停状态：是否已开始关停，以及已进门、响应还没结束的 MCP 请求数。
#[derive(Default)]
pub(crate) struct Lifecycle {
    phase: Mutex<Phase>,
    changed: Notify,
}

/// 一个已进门的请求；响应体结束或请求中途失败时随 `Drop` 注销。
pub(crate) struct InFlight(Arc<Lifecycle>);

impl Lifecycle {
    fn phase(&self) -> MutexGuard<'_, Phase> {
        // 临界区只做计数，持锁时不会 panic；万一中毒也沿用里面的值，不让关停卡在一把锁上。
        self.phase.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 关停开始前登记一个进门的请求；已开始关停就返回 `None`，调用方回 503。
    pub(crate) fn enter(self: &Arc<Self>) -> Option<InFlight> {
        let mut phase = self.phase();
        if phase.stopping {
            return None;
        }
        phase.in_flight += 1;
        Some(InFlight(self.clone()))
    }

    /// 是否已开始关停。
    pub(crate) fn is_stopping(&self) -> bool {
        self.phase().stopping
    }

    /// 开始关停：此后 `enter` 一律拒绝。可重复调用。
    pub(crate) fn stop(&self) {
        self.phase().stopping = true;
        self.changed.notify_waiters();
    }

    /// 等到开始关停。
    pub(crate) async fn stopped(&self) {
        self.wait_until(|phase| phase.stopping).await;
    }

    /// 等到开始关停、且已进门的请求全部结束。
    pub(crate) async fn drained(&self) {
        self.wait_until(|phase| phase.stopping && phase.in_flight == 0)
            .await;
    }

    async fn wait_until(&self, done: impl Fn(&Phase) -> bool) {
        loop {
            // 先建等待再检查：notify_waiters 只唤醒已经建好的等待，检查之后才建会漏掉这次变化。
            let notified = self.changed.notified();
            if done(&self.phase()) {
                return;
            }
            notified.await;
        }
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        {
            let mut phase = self.0.phase();
            phase.in_flight = phase.in_flight.saturating_sub(1);
        }
        self.0.changed.notify_waiters();
    }
}

/// 停止信号：SIGTERM（systemd 停服）与 SIGINT（Ctrl-C）等价。
///
/// 必须在宣布监听之前装好：装好之前收到的 SIGTERM 按默认动作直接杀掉进程，优雅关停根本不会发生。
/// tokio 的 `ctrl_c()` 要等第一次 poll 才装处理器，所以两路都用 `signal()` 当场装。
/// 装好之后到关停 future 被 poll 之前收到的信号由 tokio 记下，poll 时立即生效。
pub(crate) struct Signals {
    #[cfg(unix)]
    terminate: Option<tokio::signal::unix::Signal>,
    #[cfg(unix)]
    interrupt: Option<tokio::signal::unix::Signal>,
}

impl Signals {
    /// 当场装好两路处理器。某一路装不上时只是那一路不触发、进程照常服务（同原先「SIGTERM 注册失败时只等 Ctrl-C」）。
    pub(crate) fn listen() -> Self {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Self {
                terminate: signal(SignalKind::terminate()).ok(),
                interrupt: signal(SignalKind::interrupt()).ok(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {}
        }
    }

    /// 等到任一路信号。
    pub(crate) async fn received(self) {
        #[cfg(unix)]
        {
            let (mut terminate, mut interrupt) = (self.terminate, self.interrupt);
            tokio::select! {
                () = once(terminate.as_mut()) => {}
                () = once(interrupt.as_mut()) => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(unix)]
async fn once(signal: Option<&mut tokio::signal::unix::Signal>) {
    match signal {
        Some(signal) => {
            let _ = signal.recv().await;
        }
        None => std::future::pending().await,
    }
}

#[cfg(test)]
#[path = "../tests/shutdown_private/lifecycle.rs"]
mod tests;
