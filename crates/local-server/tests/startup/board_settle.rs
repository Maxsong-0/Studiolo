//! board 收尾契约：租约巡检或监听任务的出错，不因 `select!` 选中哪一支而被吞掉；
//! board 一侧先结束时把故障报给外层，并等内层服务结束才返回。
use super::{StartupError, settle_board};
use std::{
    sync::{
        Arc, Mutex,
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
        let result = settle_board(
            async { Ok(()) },
            maintenance,
            worker,
            stop,
            unheard(),
            |_| {},
        )
        .await;
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
        let result = settle_board(
            async { Ok(()) },
            maintenance,
            worker,
            stop,
            unheard(),
            |_| {},
        )
        .await;
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
            let result = settle_board(
                async { Ok(()) },
                maintenance,
                worker,
                stop,
                unheard(),
                |_| {},
            )
            .await;
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
            let settled = settle_board(inner, maintenance, worker, stop, fault, |_| {});
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
    let result = settle_board(
        async { Ok(()) },
        maintenance,
        worker,
        stop,
        unheard(),
        |_| {},
    )
    .await;
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
        |_| {},
    )
    .await;
    assert!(
        matches!(result, Err(StartupError::Listen)),
        "内层自己的错误要原样带出"
    );
}

/// 收下收尾打的日志行，用例照它核对打了哪几行（关停顺序用例也用）。
#[derive(Clone, Default)]
pub(super) struct Lines(Arc<Mutex<Vec<String>>>);

impl Lines {
    pub(super) fn sink(&self) -> impl Fn(String) + Send + Sync + use<> {
        let lines = Arc::clone(&self.0);
        move |line| lines.lock().unwrap().push(line)
    }
    pub(super) fn taken(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

/// 内层等到故障报上来才结束：board 一侧先结束是确定的，不靠 `select!` 碰运气。
fn awaiting_fault() -> (
    oneshot::Sender<()>,
    impl Future<Output = Result<(), StartupError>> + use<>,
) {
    let (fault, faulted) = oneshot::channel();
    (fault, async move {
        faulted.await.map_err(|_| StartupError::Listen)
    })
}

/// 起一个当场 panic 的任务，等它结束再交出句柄。
async fn panicked<E: Send + 'static>() -> JoinHandle<Result<(), E>> {
    let handle = tokio::spawn(async { panic!("巡检 panic 夹具") });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !handle.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    handle
}

/// board 一侧谁先结束、为什么，打一行（BUG-050：169 上人侧退出时日志里只有「HTTP服务已停止」，
/// 看不出是租约巡检先停的、停的原因是什么）。原因只有固定类别：巡检带自己的固定原因，
/// 监听的底层错误可能带路径，只记「出错」。
#[tokio::test]
async fn the_board_side_that_ended_first_and_why_is_logged() {
    let lines = Lines::default();
    let cases = [
        ("巡检出错", "看板一侧先结束：租约巡检（审计不可用）"),
        ("巡检正常结束", "看板一侧先结束：租约巡检（正常结束）"),
        ("巡检 panic", "看板一侧先结束：租约巡检（异常退出）"),
        ("监听出错", "看板一侧先结束：看板监听（出错）"),
    ];
    for (case, expected) in cases {
        let (stop, stopped) = watch::channel(false);
        let (maintenance, worker) = match case {
            "巡检出错" => (
                finished(Err("审计不可用".to_owned())).await,
                until_stopped(stopped, Ok::<(), &str>(())),
            ),
            "巡检正常结束" => (
                finished(Ok::<(), String>(())).await,
                until_stopped(stopped, Ok(())),
            ),
            "巡检 panic" => (panicked::<String>().await, until_stopped(stopped, Ok(()))),
            _ => (
                until_stopped(stopped, Ok::<(), String>(())),
                finished(Err("监听失败夹具 /私有/路径")).await,
            ),
        };
        let (fault, inner) = awaiting_fault();
        let settled = settle_board(inner, maintenance, worker, stop, fault, lines.sink());
        let result = tokio::time::timeout(WAIT, settled)
            .await
            .expect("board 一侧先结束后收尾卡住了");
        assert!(matches!(result, Err(StartupError::Serve)), "{case}");
        assert_eq!(lines.taken(), [expected], "{case}");
    }
}

/// 内层先结束、收尾途中 board 一侧出错：打「收尾出错」一行；三方都正常收尾不打。
#[tokio::test]
async fn errors_during_settle_are_logged_and_a_clean_settle_is_quiet() {
    let lines = Lines::default();
    let (stop, stopped) = watch::channel(false);
    let maintenance = until_stopped(stopped.clone(), Err("租约对账失败".to_owned()));
    let worker = until_stopped(stopped, Err("监听失败夹具 /私有/路径"));
    let result = settle_board(
        async { Ok(()) },
        maintenance,
        worker,
        stop,
        unheard(),
        lines.sink(),
    )
    .await;
    assert!(matches!(result, Err(StartupError::Serve)));
    let mut logged = lines.taken();
    logged.sort();
    assert_eq!(
        logged,
        [
            "看板收尾出错：看板监听（出错）",
            "看板收尾出错：租约巡检（租约对账失败）",
        ]
    );

    let (stop, stopped) = watch::channel(false);
    let maintenance = until_stopped(stopped.clone(), Ok::<(), String>(()));
    let worker = until_stopped(stopped, Ok::<(), &str>(()));
    let result = settle_board(
        async { Ok(()) },
        maintenance,
        worker,
        stop,
        unheard(),
        lines.sink(),
    )
    .await;
    assert!(result.is_ok());
    assert!(lines.taken().is_empty(), "三方都正常收尾不该打日志");
}
