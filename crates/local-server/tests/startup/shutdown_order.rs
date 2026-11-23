//! 停机顺序契约：不论哪一支先到都先停备份；Bridge 收尾时写者与 HTTP 仍可用，异常退出也排空 HTTP；
//! 停备份途中驱动失败或监听失败，都不能把它截断。
use super::{StartupError, board_settle::Lines, drain};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};

type Events = Arc<Mutex<Vec<&'static str>>>;
fn record(events: &Events, event: &'static str) {
    events.lock().unwrap().push(event);
}

/// 每一步等待的上限。
const WAIT: Duration = Duration::from_secs(2);
/// 备份没放开时再看这么久：这段时间里收尾链不许往下走。
const HOLD: Duration = Duration::from_millis(300);

/// 立刻做完的停备份，只记一笔。
fn backup_stops(events: &Events) -> impl Future<Output = ()> + use<> {
    let logged = events.clone();
    async move { record(&logged, "backup-stopped") }
}

/// 停备份途中卡住，等测试放行；开始时经 `started` 报一声。
fn backup_held(
    events: &Events,
    started: oneshot::Sender<()>,
    released: oneshot::Receiver<()>,
) -> impl Future<Output = ()> + use<> {
    let logged = events.clone();
    async move {
        record(&logged, "backup-start");
        let _ = started.send(());
        let _ = released.await;
        record(&logged, "backup-done");
    }
}

#[tokio::test]
async fn shutdown_stops_backup_then_waits_for_bridge_before_writer_and_http() {
    let events = Events::default();
    let (stop, mut stopped) = watch::channel(false);
    let (http_stop, http_stopped) = oneshot::channel();
    let (shutdown, requested) = oneshot::channel();
    let (backup_started, mut backup_starting) = oneshot::channel();
    let (backup_release, backup_released) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let (at_stop, mut reached) = mpsc::unbounded_channel();
    let logged = events.clone();
    let driver = tokio::spawn(async move {
        stopped.changed().await.unwrap();
        record(&logged, "bridge-stop");
        at_stop.send(()).unwrap();
        released.await.unwrap();
        record(&logged, "bridge-done");
        Ok(())
    });
    let logged = events.clone();
    let server = async move {
        http_stopped.await.unwrap();
        record(&logged, "http-drained");
        Ok(())
    };
    let logged = events.clone();
    let run = tokio::spawn(drain(
        server,
        async {
            let _ = requested.await;
        },
        backup_held(&events, backup_started, backup_released),
        Some(driver),
        stop,
        http_stop,
        move || record(&logged, "writer-stop"),
        |_| {},
    ));
    shutdown.send(()).unwrap();
    tokio::time::timeout(WAIT, &mut backup_starting)
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(HOLD, reached.recv()).await.is_err(),
        "备份还没放开就通知了 Bridge"
    );
    backup_release.send(()).unwrap();
    tokio::time::timeout(WAIT, reached.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        ["backup-start", "backup-done", "bridge-stop"]
    );
    release.send(()).unwrap();
    assert!(run.await.unwrap().is_ok());
    assert_eq!(
        *events.lock().unwrap(),
        [
            "backup-start",
            "backup-done",
            "bridge-stop",
            "bridge-done",
            "writer-stop",
            "http-drained"
        ]
    );
}

#[tokio::test]
async fn driver_failure_still_stops_backup_then_writer_and_drains_http() {
    let events = Events::default();
    let (stop, _) = watch::channel(false);
    let (http_stop, http_stopped) = oneshot::channel();
    let driver = tokio::spawn(async { Err("失败夹具".to_owned()) });
    let logged = events.clone();
    let server = async move {
        http_stopped.await.unwrap();
        record(&logged, "http-drained");
        Ok(())
    };
    let logged = events.clone();
    let result = drain(
        server,
        std::future::pending(),
        backup_stops(&events),
        Some(driver),
        stop,
        http_stop,
        move || record(&logged, "writer-stop"),
        |_| {},
    )
    .await;
    assert!(matches!(result, Err(StartupError::Serve)));
    assert_eq!(
        *events.lock().unwrap(),
        ["backup-stopped", "writer-stop", "http-drained"]
    );
}

#[tokio::test]
async fn listener_failure_reaps_driver_and_preserves_the_original_error() {
    let events = Events::default();
    let (stop, mut stopped) = watch::channel(false);
    let (http_stop, _) = oneshot::channel();
    let logged = events.clone();
    let driver = tokio::spawn(async move {
        stopped.changed().await.unwrap();
        record(&logged, "bridge-done");
        Ok(())
    });
    let logged = events.clone();
    let result = drain(
        async { Err(StartupError::Listen) },
        std::future::pending(),
        backup_stops(&events),
        Some(driver),
        stop,
        http_stop,
        move || record(&logged, "writer-stop"),
        |_| {},
    )
    .await;
    assert!(matches!(result, Err(StartupError::Listen)));
    assert_eq!(
        *events.lock().unwrap(),
        ["backup-stopped", "bridge-done", "writer-stop"]
    );
}

#[tokio::test]
async fn driver_failure_during_backup_stop_does_not_cut_it_short() {
    let events = Events::default();
    let (stop, _) = watch::channel(false);
    let (http_stop, http_stopped) = oneshot::channel();
    let (shutdown, requested) = oneshot::channel();
    let (started, mut starting) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let (fail, failing) = oneshot::channel::<()>();
    // 握手：同一次 poll 里先报一声再返回，测试收到时驱动任务已经结束。
    let (failed, mut failure_seen) = oneshot::channel::<()>();
    let driver = tokio::spawn(async move {
        let _ = failing.await;
        let _ = failed.send(());
        Err("失败夹具".to_owned())
    });
    let logged = events.clone();
    let server = async move {
        http_stopped.await.unwrap();
        record(&logged, "http-drained");
        Ok(())
    };
    let (writer_stopping, mut writer_stopped) = mpsc::unbounded_channel();
    let logged = events.clone();
    let mut run = tokio::spawn(drain(
        server,
        async {
            let _ = requested.await;
        },
        backup_held(&events, started, released),
        Some(driver),
        stop,
        http_stop,
        move || {
            record(&logged, "writer-stop");
            let _ = writer_stopping.send(());
        },
        |_| {},
    ));
    shutdown.send(()).unwrap();
    tokio::time::timeout(WAIT, &mut starting)
        .await
        .unwrap()
        .unwrap();
    fail.send(()).unwrap();
    tokio::time::timeout(WAIT, &mut failure_seen)
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(HOLD, writer_stopped.recv())
            .await
            .is_err(),
        "驱动在停备份途中失败，备份还没放开就停了刷新"
    );
    release.send(()).unwrap();
    let result = tokio::time::timeout(WAIT, &mut run).await.unwrap().unwrap();
    assert!(matches!(result, Err(StartupError::Serve)), "驱动失败要报错");
    assert_eq!(
        *events.lock().unwrap(),
        ["backup-start", "backup-done", "writer-stop", "http-drained"]
    );
}

#[tokio::test]
async fn listener_failure_during_backup_stop_does_not_cut_it_short() {
    let events = Events::default();
    let (stop, _) = watch::channel(false);
    let (http_stop, _) = oneshot::channel();
    let (shutdown, requested) = oneshot::channel();
    let (started, mut starting) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let (fail, failing) = oneshot::channel::<()>();
    // 握手：服务只在 `drain` 轮询它时才往下走，测试收到这一声就说明停备份期间服务确实被轮询、已经结束。
    let (failed, mut failure_seen) = oneshot::channel::<()>();
    let logged = events.clone();
    let server = async move {
        let _ = failing.await;
        record(&logged, "listener-failed");
        let _ = failed.send(());
        Err(StartupError::Listen)
    };
    let (writer_stopping, mut writer_stopped) = mpsc::unbounded_channel();
    let logged = events.clone();
    let mut run = tokio::spawn(drain(
        server,
        async {
            let _ = requested.await;
        },
        backup_held(&events, started, released),
        None,
        stop,
        http_stop,
        move || {
            record(&logged, "writer-stop");
            let _ = writer_stopping.send(());
        },
        |_| {},
    ));
    shutdown.send(()).unwrap();
    tokio::time::timeout(WAIT, &mut starting)
        .await
        .unwrap()
        .unwrap();
    fail.send(()).unwrap();
    tokio::time::timeout(WAIT, &mut failure_seen)
        .await
        .expect("停备份期间服务没被轮询，监听失败要等到最后才发现")
        .unwrap();
    assert!(
        tokio::time::timeout(HOLD, writer_stopped.recv())
            .await
            .is_err(),
        "监听在停备份途中失败，备份还没放开就停了刷新"
    );
    release.send(()).unwrap();
    let result = tokio::time::timeout(WAIT, &mut run).await.unwrap().unwrap();
    assert!(
        matches!(result, Err(StartupError::Listen)),
        "监听自己的错误要原样带出"
    );
    assert_eq!(
        *events.lock().unwrap(),
        [
            "backup-start",
            "listener-failed",
            "backup-done",
            "writer-stop"
        ]
    );
}

/// 关停链从哪一支开始的、收尾时谁出错，各打一行（BUG-050：169 上人侧退出时日志里看不出起因）。
/// 同步驱动的错误可能带路径，只记类别；HTTP 服务的错误是固定说法，照原样带上。
#[tokio::test]
async fn the_cause_of_shutdown_is_logged() {
    let lines = Lines::default();

    // 同步驱动先结束（出错、异常退出）。
    for (driver, expected) in [
        (
            tokio::spawn(async { Err("失败夹具 /私有/路径".to_owned()) }),
            "关停起因：同步驱动先结束（出错）",
        ),
        (
            tokio::spawn(async { panic!("驱动 panic 夹具") }),
            "关停起因：同步驱动先结束（异常退出）",
        ),
    ] {
        let (stop, _) = watch::channel(false);
        let (http_stop, http_stopped) = oneshot::channel::<()>();
        let server = async move {
            let _ = http_stopped.await;
            Ok(())
        };
        let result = drain(
            server,
            std::future::pending(),
            async {},
            Some(driver),
            stop,
            http_stop,
            || {},
            lines.sink(),
        )
        .await;
        assert!(matches!(result, Err(StartupError::Serve)));
        assert_eq!(lines.taken(), [expected]);
    }

    // HTTP 服务先结束（出错）：带它自己的固定说法。
    let (stop, _) = watch::channel(false);
    let (http_stop, _) = oneshot::channel();
    let result = drain(
        async { Err(StartupError::Listen) },
        std::future::pending(),
        async {},
        None,
        stop,
        http_stop,
        || {},
        lines.sink(),
    )
    .await;
    assert!(matches!(result, Err(StartupError::Listen)));
    assert_eq!(
        lines.taken(),
        [format!(
            "关停起因：HTTP服务先结束（{}）",
            StartupError::Listen
        )]
    );

    // 关停信号先到（退出信号或看板一侧故障）；收尾时同步驱动出错，另打一行。
    let (stop, mut stopped) = watch::channel(false);
    let (http_stop, http_stopped) = oneshot::channel::<()>();
    let driver = tokio::spawn(async move {
        let _ = stopped.changed().await;
        Err::<(), String>("失败夹具 /私有/路径".to_owned())
    });
    let server = async move {
        let _ = http_stopped.await;
        Ok(())
    };
    let result = drain(
        server,
        async {},
        async {},
        Some(driver),
        stop,
        http_stop,
        || {},
        lines.sink(),
    )
    .await;
    assert!(matches!(result, Err(StartupError::Serve)));
    assert_eq!(
        lines.taken(),
        [
            "关停起因：退出信号或看板一侧先结束",
            "关停收尾出错：同步驱动（出错）",
        ]
    );
}
