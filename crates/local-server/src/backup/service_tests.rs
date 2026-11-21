//! 接受循环的关停顺序（T-216 r17 第 3 轮独立复查 R3-F7）：停止信号到达时正在处理的连接照旧做完；
//! 之后 backlog 里排着的连接不再接，到点的回收也不再起。发值和丢掉发送端两种停法都要成立。
use super::accept_until_stopped;
use std::{
    os::unix::net::{UnixListener, UnixStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
use tokio::{runtime::Runtime, sync::watch};

/// 不按顺序看分支时，停止信号、到点的回收、排着的连接三路同时就绪，每轮只有约三分之一的机会碰巧先停；
/// 这么多轮全都碰巧过的概率约 3^-40，可以忽略。
const ROUNDS: usize = 40;
/// 测试里的回收周期；在途连接要握得比它久，停止信号到达时回收一定已经到点。
const RECLAIM_EVERY: Duration = Duration::from_millis(5);
const HOLD: Duration = Duration::from_millis(25);
/// 等在途连接被接下、等循环退出的上限；超过就判失败，不让测试挂住。
const LIMIT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug)]
enum Stop {
    /// 正常关停：`BackupService::shutdown` 发 `true`。
    Send,
    /// 发送端没了：`changed()` 返回错误。
    DropSender,
}

#[test]
fn stop_wins_over_queued_connection_and_due_reclaim() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    for round in 0..ROUNDS {
        let stop = if round % 2 == 0 {
            Stop::Send
        } else {
            Stop::DropSender
        };
        one_round(&runtime, round, stop);
    }
}

/// 一轮：接下第一个连接并在后台线程里握住它；握住期间回收到点、第二个连接排进 backlog、停止信号到达；
/// 再放开第一个连接。循环必须直接退出：处理过的连接只有第一个，回收次数停在放开之前的数。
fn one_round(runtime: &Runtime, round: usize, stop_by: Stop) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("backup.sock");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();

    let reclaims = Arc::new(AtomicUsize::new(0));
    let served = Arc::new(AtomicUsize::new(0));
    let (started, started_seen) = mpsc::channel::<()>();
    let started = Mutex::new(started);
    let (release, released) = mpsc::channel::<()>();
    let released = Mutex::new(released);
    let (stop, stopped) = watch::channel(false);

    let acceptor = {
        let _context = runtime.enter();
        let listener = tokio::net::UnixListener::from_std(listener).unwrap();
        let (reclaims, served) = (Arc::clone(&reclaims), Arc::clone(&served));
        runtime.spawn(accept_until_stopped(
            listener,
            stopped,
            RECLAIM_EVERY,
            move || {
                reclaims.fetch_add(1, Ordering::SeqCst);
            },
            move |_stream| {
                served.fetch_add(1, Ordering::SeqCst);
                let _ = started.lock().unwrap().send(());
                // 握到测试放开（放开＝丢掉发送端），之后不该有的连接也不会卡在这里。
                let _ = released.lock().unwrap().recv();
            },
        ))
    };

    let _in_flight = UnixStream::connect(&path).unwrap();
    started_seen
        .recv_timeout(LIMIT)
        .unwrap_or_else(|_| panic!("第 {round} 轮：在途连接没有被接下"));
    // 循环一次只做一件事：第一个连接握着，回收不会再跑，这个数到放开之前都不变。
    let reclaims_before_stop = reclaims.load(Ordering::SeqCst);
    std::thread::sleep(HOLD);
    let _queued = UnixStream::connect(&path).unwrap();
    let _kept = match stop_by {
        Stop::Send => {
            stop.send(true).unwrap();
            Some(stop)
        }
        Stop::DropSender => {
            drop(stop);
            None
        }
    };
    drop(release);

    let finished = runtime.block_on(async { tokio::time::timeout(LIMIT, acceptor).await });
    assert!(
        matches!(finished, Ok(Ok(()))),
        "第 {round} 轮（{stop_by:?}）：停止信号之后接受循环没有在期限内正常退出"
    );
    assert_eq!(
        served.load(Ordering::SeqCst),
        1,
        "第 {round} 轮（{stop_by:?}）：停止信号之后又接了 backlog 里排着的连接"
    );
    assert_eq!(
        reclaims.load(Ordering::SeqCst),
        reclaims_before_stop,
        "第 {round} 轮（{stop_by:?}）：停止信号之后又起了一次到点的回收"
    );
}
