//! 关停状态的计数与唤醒：进门登记、开始关停后拒绝、排空要等最后一个请求注销。
use super::Lifecycle;
use std::{sync::Arc, time::Duration};

#[test]
fn requests_entering_after_stop_are_refused() {
    let lifecycle = Arc::new(Lifecycle::default());
    let before = lifecycle.enter();
    assert!(before.is_some());
    assert!(!lifecycle.is_stopping());
    lifecycle.stop();
    assert!(lifecycle.is_stopping());
    assert!(lifecycle.enter().is_none(), "开始关停后不能再登记新请求");
    // 重复 stop 不改变结论。
    lifecycle.stop();
    assert!(lifecycle.enter().is_none());
}

#[tokio::test]
async fn drained_waits_for_the_last_request_that_entered_before_stop() {
    let lifecycle = Arc::new(Lifecycle::default());
    let first = lifecycle.enter().expect("关停前可以登记");
    let second = lifecycle.enter().expect("关停前可以登记");
    lifecycle.stop();
    tokio::time::timeout(Duration::from_secs(1), lifecycle.stopped())
        .await
        .expect("已开始关停");
    let waiter = {
        let lifecycle = lifecycle.clone();
        tokio::spawn(async move { lifecycle.drained().await })
    };
    drop(first);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!waiter.is_finished(), "还有一个请求没结束，不能算排空");
    drop(second);
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("最后一个请求注销后立即排空")
        .expect("等待任务");
}

#[tokio::test]
async fn drained_needs_stop_even_when_nothing_is_in_flight() {
    let lifecycle = Arc::new(Lifecycle::default());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), lifecycle.drained())
            .await
            .is_err(),
        "没开始关停就不算排空"
    );
    let waiter = {
        let lifecycle = lifecycle.clone();
        tokio::spawn(async move { lifecycle.drained().await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    lifecycle.stop();
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("开始关停时没有请求在途，立即排空")
        .expect("等待任务");
}
