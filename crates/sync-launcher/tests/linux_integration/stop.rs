//! 主进程退出而后代仍活时，Stop 必须停止整组并保留等待身份。
use super::support::{
    assert_retained, call, cleanup_unit, fixture, launch_frame, user_manager, wait_unpopulated,
};
use super::{PAYLOAD, UNVERIFIED};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use studiolo_sync_launcher::{
    protocol::{Empty, Request, Response, StopFrame},
    serve::serve_with,
    unit::Manager,
};
use studiolo_sync_transport::{linux::service, owned_run::RunOperation};

#[test]
#[ignore = "平台契约未验：需要可用的 systemd --user；用 --include-ignored 真跑"]
fn reaped主进程仍有后代时stop保留证据直到空组() {
    let runtime = user_manager().expect(UNVERIFIED);
    let fixture = fixture(&runtime);
    fs::write(&fixture.runtime.cli_path, "/bin/sleep 30 &\nexit 0\n").unwrap();
    let (config, layout) = (fixture.config.clone(), fixture.layout.clone());
    thread::spawn(move || {
        serve_with(config, Manager::user(PathBuf::from(PAYLOAD)), &layout).unwrap()
    });
    let socket = &fixture.layout.socket;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !socket.exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
    let argv =
        service::build_argv(&fixture.runtime, RunOperation::RunOnce, &fixture.mirror).unwrap();
    let request = launch_frame(&fixture.config, 1, service::argv_digest(&argv));
    let container = match call(socket, &Request::Launch(request)) {
        Response::Launch(report) => report.container,
        other => panic!("launch: {other:?}"),
    };
    assert_eq!(
        call(socket, &Request::Release(container.clone())),
        Response::Release(Empty {})
    );
    assert_eq!(
        call(socket, &Request::Gate(container.clone())),
        Response::Gate(Empty {})
    );
    loop {
        match call(socket, &Request::Reap(container.clone())) {
            Response::Reap {
                exit_observed: true,
                exit_code: Some(0),
                exit_signal: None,
            } => break,
            Response::Reap {
                exit_observed: false,
                ..
            } => {}
            other => panic!("reap: {other:?}"),
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        matches!(
            call(socket, &Request::Events(container.clone())),
            Response::Events {
                populated: true,
                ..
            }
        ),
        "主进程已退出，后代仍在"
    );
    assert_eq!(
        call(
            socket,
            &Request::Stop(StopFrame {
                container: container.clone(),
                grace_seconds: 1
            })
        ),
        Response::Stop(Empty {})
    );
    wait_unpopulated(socket, &container);
    assert_retained(socket, &container, 0);
    cleanup_unit(&container.unit_name);
}
