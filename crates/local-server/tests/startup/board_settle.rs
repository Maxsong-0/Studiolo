//! board 收尾契约：租约巡检或监听任务的出错，不因 `select!` 选中哪一支而被吞掉。
use super::{StartupError, settle_board};
use std::time::Duration;
use tokio::{sync::watch, task::JoinHandle};

/// `select!` 每次从随机一支开始轮询，同时就绪的几支都可能被选中；
/// 多轮把「每轮都恰好选中出错那支、漏掉缺陷」的概率压到 (1/3)^40，可以忽略。
const ROUNDS: usize = 40;

/// 起一个立刻返回 `outcome` 的任务，等它真正结束再交出句柄：交进收尾时它已经就绪。
async fn finished<E: Send + 'static>(outcome: Result<(), E>) -> JoinHandle<Result<(), E>> {
    let handle = tokio::spawn(async move { outcome });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !handle.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    handle
}

/// 起一个收到停止通知才返回 `outcome` 的任务，模拟巡检或监听在收尾途中结束。
fn until_stopped<E: Send + 'static>(
    mut stopped: watch::Receiver<bool>,
    outcome: Result<(), E>,
) -> JoinHandle<Result<(), E>> {
    tokio::spawn(async move {
        let _ = stopped.changed().await;
        outcome
    })
}

#[tokio::test]
async fn maintenance_error_ready_alongside_shutdown_is_reported() {
    for round in 0..ROUNDS {
        let (stop, stopped) = watch::channel(false);
        let maintenance = finished(Err::<(), _>("巡检失败夹具".to_owned())).await;
        let worker = until_stopped(stopped, Ok::<(), &str>(()));
        let result = settle_board(async { Ok(()) }, maintenance, worker, stop).await;
        assert!(
            matches!(result, Err(StartupError::Serve)),
            "第 {round} 轮：巡检出错和关停同时就绪，出错被吞"
        );
    }
}

#[tokio::test]
async fn worker_error_ready_alongside_shutdown_is_reported() {
    for round in 0..ROUNDS {
        let (stop, stopped) = watch::channel(false);
        let maintenance = until_stopped(stopped, Ok::<(), String>(()));
        let worker = finished(Err::<(), _>("监听失败夹具")).await;
        let result = settle_board(async { Ok(()) }, maintenance, worker, stop).await;
        assert!(
            matches!(result, Err(StartupError::Serve)),
            "第 {round} 轮：监听出错和关停同时就绪，出错被吞"
        );
    }
}

#[tokio::test]
async fn errors_finishing_during_settle_are_reported() {
    for round in 0..ROUNDS {
        for (maintenance_fails, worker_fails) in [(true, false), (false, true)] {
            let (stop, stopped) = watch::channel(false);
            let maintenance = until_stopped(
                stopped.clone(),
                if maintenance_fails {
                    Err("巡检失败夹具".to_owned())
                } else {
                    Ok(())
                },
            );
            let worker = until_stopped(
                stopped,
                if worker_fails {
                    Err("监听失败夹具")
                } else {
                    Ok(())
                },
            );
            let result = settle_board(async { Ok(()) }, maintenance, worker, stop).await;
            assert!(
                matches!(result, Err(StartupError::Serve)),
                "第 {round} 轮：收到停止后才出错（巡检 {maintenance_fails}，监听 {worker_fails}），出错被吞"
            );
        }
    }
}

#[tokio::test]
async fn early_exit_wins_without_polling_its_handle_twice() {
    // 巡检或监听在关停前就结束（不论成败）都算异常；赢了 select 的句柄不能再 poll 第二遍。
    for outcome in [Ok(()), Err("提前结束夹具".to_owned())] {
        let (stop, stopped) = watch::channel(false);
        let maintenance = finished(outcome.clone()).await;
        let worker = until_stopped(stopped, Ok::<(), &str>(()));
        let result = settle_board(std::future::pending(), maintenance, worker, stop).await;
        assert!(
            matches!(result, Err(StartupError::Serve)),
            "巡检提前结束：{outcome:?}"
        );

        let (stop, stopped) = watch::channel(false);
        let maintenance = until_stopped(stopped, Ok::<(), String>(()));
        let worker = finished(outcome.clone()).await;
        let result = settle_board(std::future::pending(), maintenance, worker, stop).await;
        assert!(
            matches!(result, Err(StartupError::Serve)),
            "监听提前结束：{outcome:?}"
        );
    }
}

#[tokio::test]
async fn clean_shutdown_stays_ok_and_inner_error_is_kept() {
    let (stop, stopped) = watch::channel(false);
    let maintenance = until_stopped(stopped.clone(), Ok::<(), String>(()));
    let worker = until_stopped(stopped, Ok::<(), &str>(()));
    let result = settle_board(async { Ok(()) }, maintenance, worker, stop).await;
    assert!(result.is_ok(), "三方都正常收尾不能报错");

    let (stop, stopped) = watch::channel(false);
    let maintenance = until_stopped(stopped.clone(), Ok::<(), String>(()));
    let worker = until_stopped(stopped, Ok::<(), &str>(()));
    let result = settle_board(
        async { Err(StartupError::Listen) },
        maintenance,
        worker,
        stop,
    )
    .await;
    assert!(
        matches!(result, Err(StartupError::Listen)),
        "内层自己的错误要原样带出"
    );
}
