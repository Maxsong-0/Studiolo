//! board 一侧先退出的收尾契约：内层服务不被丢弃，board 故障照退出信号走完整收尾链
//! （停备份 → 等 Bridge 收尾 → 停刷新 → 排空 HTTP），在途请求做完、`drain` 返回之后才轮到
//! 等写者收尾和进程退出，退出状态是错误。
use super::{StartupError, drain, settle_board, shutdown_requested};
use axum::{Router, routing::get};
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};

type Events = Arc<Mutex<Vec<&'static str>>>;
fn record(events: &Events, event: &'static str) {
    events.lock().unwrap().push(event);
}

/// 每一步等待的上限；超过就算卡死。
const WAIT: Duration = Duration::from_secs(10);
/// 停刷新之后再看这么久：修复前收尾链此时早已走完，修复后在途请求不放行就走不完。
const HOLD: Duration = Duration::from_millis(300);

async fn within<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(WAIT, future)
        .await
        .expect("等待超时，收尾链卡住了")
}

/// 收到 board 停止通知就正常结束的一方。
fn until_stopped(mut stopped: watch::Receiver<bool>) -> JoinHandle<Result<(), String>> {
    tokio::spawn(async move {
        let _ = stopped.changed().await;
        Ok(())
    })
}

/// board 一侧先出错的是哪一方。
#[derive(Clone, Copy, Debug)]
enum Side {
    Maintenance,
    Worker,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maintenance_exit_drains_in_flight_request_before_writer_cleanup() {
    board_exit_drains_in_flight_request(Side::Maintenance).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_exit_drains_in_flight_request_before_writer_cleanup() {
    board_exit_drains_in_flight_request(Side::Worker).await;
}

async fn board_exit_drains_in_flight_request(side: Side) {
    let events = Events::default();

    // 内层：真实 axum 服务，和生产内层一样带优雅关停；请求进门后卡住，等测试放行。
    let (entered, mut entering) = mpsc::unbounded_channel();
    let (release, released) = watch::channel(false);
    let logged = events.clone();
    let app = Router::new().route(
        "/slow",
        get(move || {
            let (logged, entered, mut released) =
                (logged.clone(), entered.clone(), released.clone());
            async move {
                record(&logged, "request-start");
                let _ = entered.send(());
                let _ = released.wait_for(|go| *go).await;
                record(&logged, "request-done");
                "done"
            }
        }),
    );
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (http_stop, http_stopped) = oneshot::channel::<()>();
    let logged = events.clone();
    let inner = async move {
        let served = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = http_stopped.await;
            })
            .await
            .map_err(|_| StartupError::Serve);
        record(&logged, "http-drained");
        served
    };

    // board 一侧：出错的一方等测试点火，另一方收到 board 停止通知就正常结束。
    let (board_stop, board_stopped) = watch::channel(false);
    let (ignite, ignited) = oneshot::channel::<()>();
    let failing = tokio::spawn(async move {
        let _ = ignited.await;
        Err::<(), String>("board 一侧失败夹具".to_owned())
    });
    let healthy = until_stopped(board_stopped);
    let (maintenance, worker) = match side {
        Side::Maintenance => (failing, healthy),
        Side::Worker => (healthy, failing),
    };

    // Bridge 驱动：收到停止通知记一笔，用来核对 board 故障走的是和退出信号同一条链。
    let (bridge_stop, mut bridge_stopped) = watch::channel(false);
    let logged = events.clone();
    let driver = tokio::spawn(async move {
        let _ = bridge_stopped.changed().await;
        record(&logged, "bridge-done");
        Ok::<(), String>(())
    });

    // 外层照 `serve_until` 接：board 故障经 `shutdown_requested` 进关停；退出信号这里永远不来。
    let (fault, faulted) = oneshot::channel();
    let (writer_stopping, mut writer_stopped) = mpsc::unbounded_channel();
    let (logged, backup_logged, stop_logged) = (events.clone(), events.clone(), events.clone());
    let mut run = tokio::spawn(async move {
        let result = drain(
            settle_board(inner, maintenance, worker, board_stop, fault, |_| {}),
            shutdown_requested(std::future::pending(), faulted),
            async move { record(&backup_logged, "backup-stopped") },
            Some(driver),
            bridge_stop,
            http_stop,
            move || {
                record(&stop_logged, "writer-stop");
                let _ = writer_stopping.send(());
            },
            |_| {},
        )
        .await;
        // 对应 `serve_until` 在 `drain` 之后的 `canonical_input::shutdown`：等写者的清理作业做完
        // （停刷新时写者若空闲就已开始清理、放掉 Broker，之后的请求拿到「不可用」），随后进程退出。
        record(&logged, "writer-shutdown");
        result
    });

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let request = tokio::spawn(async move {
        let response = client.get(format!("http://{address}/slow")).send().await?;
        let status = response.status().as_u16();
        Ok::<_, reqwest::Error>((status, response.text().await?))
    });
    within(entering.recv()).await.expect("请求没进到内层");

    let _ = ignite.send(());
    within(writer_stopped.recv())
        .await
        .expect("board 故障没有触发关停链");
    assert!(
        tokio::time::timeout(HOLD, &mut run).await.is_err(),
        "{side:?} 先退出：收尾链没等内层在途请求排空就走完了"
    );

    release.send(true).unwrap();
    let result = within(run).await.unwrap();
    assert!(
        matches!(result, Err(StartupError::Serve)),
        "{side:?} 先退出必须以错误收场"
    );
    let (status, body) = within(request).await.unwrap().expect("在途请求被截断");
    assert_eq!((status, body.as_str()), (200, "done"));
    assert_eq!(
        *events.lock().unwrap(),
        [
            "request-start",
            "backup-stopped",
            "bridge-done",
            "writer-stop",
            "request-done",
            "http-drained",
            "writer-shutdown",
        ],
        "{side:?} 先退出：收尾顺序和退出信号那条链不一样"
    );
}

#[tokio::test]
async fn only_a_reported_fault_or_the_signal_starts_shutdown() {
    // board 一侧没报就丢下发送端（没有 board，或 board 正常收尾）不算故障：一直等退出信号。
    let (fault, faulted) = oneshot::channel::<()>();
    drop(fault);
    let (signal, signalled) = oneshot::channel::<()>();
    let requested = shutdown_requested(
        async move {
            let _ = signalled.await;
        },
        faulted,
    );
    tokio::pin!(requested);
    assert!(
        tokio::time::timeout(HOLD, &mut requested).await.is_err(),
        "board 一侧没报故障就开始关停了"
    );
    signal.send(()).unwrap();
    within(requested).await;

    // 报了故障，不等退出信号就开始关停。
    let (fault, faulted) = oneshot::channel();
    let requested = shutdown_requested(std::future::pending(), faulted);
    fault.send(()).unwrap();
    within(requested).await;
}
