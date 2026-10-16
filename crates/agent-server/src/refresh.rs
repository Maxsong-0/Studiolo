//! 后台每 60 秒重扫白名单；失败保留旧代次。

use std::{sync::Arc, time::Duration};

use crate::state::AppState;

/// 周期重扫任务；开始关停后不再开新的一轮，正在跑的那次做完才返回（BUG-033）。
pub async fn run(state: Arc<AppState>, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        tokio::select! {
            biased;
            () = state.lifecycle.stopped() => break,
            _ = ticker.tick() => {}
        }
        let job = state.clone();
        let _ = state.verifier.run(move || job.rescan()).await;
    }
}
