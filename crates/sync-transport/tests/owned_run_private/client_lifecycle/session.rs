//! `SetMode` 帧与 `RunSession` 在真实 adapter 上的全路径；从 `client_lifecycle.rs` 拆出只为压住行数。

use super::super::launcher_fake::FakeLauncher;
use super::super::support::*;
use super::{CONFIG_JSON, adapter, launch_request};
use crate::linux::protocol::Request;
use crate::linux::service::{argv_digest, build_argv};
use crate::linux::{ContainmentBackend, LinuxContainment, verify_backend_at};
use crate::owned_run::session::RunSession;
use crate::owned_run::*;
use std::path::Path;

#[test]
fn a_set_mode_launch_frame_carries_the_mode_and_its_own_argv_digest() {
    let launcher = FakeLauncher::start();
    let approval = FakeApproval::new();
    let mut adapter = adapter(&launcher, &approval);
    let request = launch_request(RunOperation::SetMode(SyncMode::Bidirectional), &approval);
    adapter.issue_launch(&request).expect("屏障可建立");
    let script = launcher.script();
    let Some(Request::Launch(frame)) = script.requests.last() else {
        panic!("最后一条应是 launch");
    };
    assert_eq!(frame.operation, "set_mode");
    assert_eq!(frame.mode, Some(SyncMode::Bidirectional));
    assert_eq!(frame.argv_digest, argv_digest(&request.argv));
    let inspect = build_argv(
        &approval.runtime,
        RunOperation::InspectConfig,
        Path::new(MIRROR_ROOT),
    )
    .expect("可以构造");
    assert_ne!(frame.argv_digest, argv_digest(&inspect));
    assert_eq!(
        &request.argv[request.argv.len() - 3..],
        &["--mode", "bidirectional", "--json"]
    );
}

#[test]
fn a_run_session_on_the_real_adapter_closes_with_its_container_and_yields_the_config_output() {
    let launcher = FakeLauncher::start();
    let fixture = Fixture::new();
    let backend =
        verify_backend_at(&fixture.approval, launcher.socket()).expect("假 launcher 探测通过");
    let mut session = RunSession::open(
        &fixture.approval,
        PinnedRuntime::from_approval(&fixture.approval).expect("固定运行时可取"),
        LinuxContainment::new(backend),
        fixture.slot.publisher(),
    )
    .expect("既有槽可打开");
    let prepared = session
        .prepare_once(1, RunOperation::InspectConfig, 1_700_000_000_000)
        .expect("可以准备一次运行");
    acknowledge(&mut session);
    let owned = session.start(&prepared).expect("Intent 已 ACK 后可以放行");
    {
        let script = launcher.script();
        let kinds = script
            .requests
            .iter()
            .map(|request| match request {
                Request::Probe { .. } => "probe",
                Request::BootId(_) => "boot_id",
                Request::Launch(_) => "launch",
                Request::Release(_) => "release",
                _ => "other",
            })
            .collect::<Vec<_>>();
        assert_eq!(kinds, ["probe", "boot_id", "launch", "release"]);
    }
    assert_eq!(
        session.wait(&owned).expect("可以推进观察").phase,
        RunPhase::Running
    );
    {
        let mut script = launcher.script();
        script.running = false;
        script.exit = (true, Some(0), None);
        script.populated = false;
        script.output = Ok(CONFIG_JSON.to_owned());
    }
    assert_eq!(
        session.wait(&owned).expect("可以推进观察"),
        RunObservation {
            phase: RunPhase::OutcomePending,
            quiescence: Quiescence::Confirmed,
            exit: ExitObservation::Code(0),
            stop_reason: None,
        }
    );
    assert!(launcher.script().gated);
    acknowledge(&mut session);
    let status = session.inspect().expect("可以读状态");
    let terminal = status.last_terminal.expect("已审计闭合摘要");
    assert_eq!(terminal.observation.phase, RunPhase::Closed);
    assert_eq!(
        terminal
            .container
            .as_ref()
            .map(|container| container.launcher_session),
        Some([9u8; 16])
    );
    assert_eq!(
        session.config_output(owned.run_id()).as_deref(),
        Ok(CONFIG_JSON)
    );
    assert!(session.quiescent().is_ok());
}
