//! board 收尾契约：租约巡检或监听任务的出错，不因 `select!` 选中哪一支而被吞掉；
//! board 一侧先结束时把故障报给外层，并等内层服务结束才返回。
use super::{StartupError, settle_board};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{oneshot, watch},
    task::JoinHandle,
};

/// `select!` 每次从随机一支开始轮询，同时就绪的几支都可能被选中；
/// 多轮把「每轮都恰好选中出错那支、漏掉缺陷」的概率压到 (1/3)^40，可以忽略。
const ROUNDS: usize = 40;
/// 收尾的等待上限：超过就算卡死（故障没报上去，内层永远等不到关停）。
const WAIT: Duration = Duration::from_secs(10);

/// 接收端已经不在的故障上报口：这些用例的内层自己会结束，用不着外层响应故障。
fn unheard() -> oneshot::Sender<()> {
    oneshot::channel().0
}

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
        let result = settle_board(async { Ok(()) }, maintenance, worker, stop, unheard()).await;
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
        let result = settle_board(async { Ok(()) }, maintenance, worker, stop, unheard()).await;
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
            let result = settle_board(async { Ok(()) }, maintenance, worker, stop, unheard()).await;
            assert!(
                matches!(result, Err(StartupError::Serve)),
                "第 {round} 轮：收到停止后才出错（巡检 {maintenance_fails}，监听 {worker_fails}），出错被吞"
            );
        }
    }
}

#[tokio::test]
async fn early_exit_reports_fault_and_waits_for_inner() {
    // 巡检或监听在关停前就结束（不论成败）都算异常：把故障报给外层、等内层结束，结果是错误；
    // 赢了 select 的句柄不能再 poll 第二遍。
    for outcome in [Ok(()), Err("提前结束夹具".to_owned())] {
        for maintenance_first in [true, false] {
            let (stop, stopped) = watch::channel(false);
            let early = finished(outcome.clone()).await;
            let waiting = until_stopped(stopped, Ok(()));
            let (maintenance, worker) = if maintenance_first {
                (early, waiting)
            } else {
                (waiting, early)
            };
            // 内层只在外层收到故障、送来关停信号之后才结束：故障没报上去，就永远等不到。
            let (fault, faulted) = oneshot::channel();
            let drained = Arc::new(AtomicBool::new(false));
            let marking = Arc::clone(&drained);
            let inner = async move {
                faulted.await.map_err(|_| StartupError::Listen)?;
                marking.store(true, Ordering::SeqCst);
                Ok(())
            };
            let settled = settle_board(inner, maintenance, worker, stop, fault);
            let result = tokio::time::timeout(WAIT, settled)
                .await
                .expect("board 一侧先结束后没把故障报给外层，内层等不到关停");
            let side = if maintenance_first {
                "巡检"
            } else {
                "监听"
            };
            assert!(
                matches!(result, Err(StartupError::Serve)),
                "{side}提前结束（{outcome:?}）：结果不是错误"
            );
            assert!(
                drained.load(Ordering::SeqCst),
                "{side}提前结束（{outcome:?}）：内层还没排空就被丢下了"
            );
        }
    }
}

#[tokio::test]
async fn clean_shutdown_stays_ok_and_inner_error_is_kept() {
    let (stop, stopped) = watch::channel(false);
    let maintenance = until_stopped(stopped.clone(), Ok::<(), String>(()));
    let worker = until_stopped(stopped, Ok::<(), &str>(()));
    let result = settle_board(async { Ok(()) }, maintenance, worker, stop, unheard()).await;
    assert!(result.is_ok(), "三方都正常收尾不能报错");

    let (stop, stopped) = watch::channel(false);
    let maintenance = until_stopped(stopped.clone(), Ok::<(), String>(()));
    let worker = until_stopped(stopped, Ok::<(), &str>(()));
    let result = settle_board(
        async { Err(StartupError::Listen) },
        maintenance,
        worker,
        stop,
        unheard(),
    )
    .await;
    assert!(
        matches!(result, Err(StartupError::Listen)),
        "内层自己的错误要原样带出"
    );
}
