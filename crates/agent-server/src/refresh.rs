//! 后台每 60 秒重扫白名单；失败保留旧代次。

use std::{sync::Arc, time::Duration};

use crate::state::AppState;

/// 周期重扫任务。
pub async fn run(state: Arc<AppState>, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let job = state.clone();
        let _ = state.verifier.run(move || job.rescan()).await;
    }
}
