//! 停机顺序契约：Bridge 收尾时写者与 HTTP 仍可用，异常退出也排空 HTTP。
use super::{StartupError, drain};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot, watch};

type Events = Arc<Mutex<Vec<&'static str>>>;
fn record(events: &Events, event: &'static str) {
    events.lock().unwrap().push(event);
}

#[tokio::test]
async fn shutdown_waits_for_bridge_before_stopping_writer_and_draining_http() {
    let events = Events::default();
    let (stop, mut stopped) = watch::channel(false);
    let (http_stop, http_stopped) = oneshot::channel();
    let (shutdown, requested) = oneshot::channel();
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
        Some(driver),
        stop,
        http_stop,
        move || record(&logged, "writer-stop"),
    ));
    shutdown.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), reached.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*events.lock().unwrap(), ["bridge-stop"]);
    release.send(()).unwrap();
    assert!(run.await.unwrap().is_ok());
    assert_eq!(
        *events.lock().unwrap(),
        ["bridge-stop", "bridge-done", "writer-stop", "http-drained"]
    );
}

#[tokio::test]
async fn driver_failure_still_stops_writer_and_drains_http() {
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
        Some(driver),
        stop,
        http_stop,
        move || record(&logged, "writer-stop"),
    )
    .await;
    assert!(matches!(result, Err(StartupError::Serve)));
    assert_eq!(*events.lock().unwrap(), ["writer-stop", "http-drained"]);
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
        Some(driver),
        stop,
        http_stop,
        move || record(&logged, "writer-stop"),
    )
    .await;
    assert!(matches!(result, Err(StartupError::Listen)));
    assert_eq!(*events.lock().unwrap(), ["bridge-done", "writer-stop"]);
}
