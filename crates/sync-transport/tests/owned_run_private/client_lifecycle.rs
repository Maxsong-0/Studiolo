//! 真实 adapter（`LinuxContainment`）经假 launcher 走完整个容器协议；`SetMode` 帧与
//! `RunSession` 在它之上闭合并取回配置输出的用例在 `client_lifecycle/session.rs`。
//! 假 launcher 只证明协议映射，不证明平台受支持。

mod session;

use super::launcher_fake::{FakeLauncher, Tamper, encode};
use super::support::*;
use crate::linux::cgroup::CgroupEvents;
use crate::linux::launcher::{HandshakeNonce, LaunchRequest, ReleaseToken};
use crate::linux::protocol::{Request, Response, Verdict};
use crate::linux::service::{argv_digest, build_argv};
use crate::linux::{
    ContainmentBackend, ContainmentGap, IdentityMatch, LinuxContainment, verify_backend_at,
};
use crate::owned_run::*;
use std::path::Path;

pub(super) const CONFIG_JSON: &str = r#"{"syncMode":"mirror-remote"}"#;

pub(super) fn adapter(launcher: &FakeLauncher, approval: &FakeApproval) -> LinuxContainment {
    let backend = verify_backend_at(approval, launcher.socket()).expect("假 launcher 探测通过");
    LinuxContainment::new(backend)
}

pub(super) fn launch_request(operation: RunOperation, approval: &FakeApproval) -> LaunchRequest {
    let argv = build_argv(&approval.runtime, operation, Path::new(MIRROR_ROOT)).expect("可以构造");
    LaunchRequest {
        run_id: RunId::from_bytes([1u8; 16]),
        sequence: 1,
        operation,
        payload_uid: PAYLOAD_UID,
        payload_gid: PAYLOAD_GID,
        budget: budget(),
        argv,
        nonce: HandshakeNonce::from_bytes([0xEF; 32]),
    }
}

#[test]
fn the_real_adapter_walks_the_whole_container_protocol_through_the_fake_launcher() {
    let launcher = FakeLauncher::start();
    let approval = FakeApproval::new();
    let mut adapter = adapter(&launcher, &approval);
    assert_eq!(adapter.boot_id(), Ok([7u8; 16]));

    let request = launch_request(RunOperation::InspectConfig, &approval);
    let run_id = request.run_id;
    let ack = adapter.issue_launch(&request).expect("屏障可建立");
    assert_eq!(
        ack.container.unit_name,
        "studiolo-sync-payload-1-01010101.service"
    );
    assert_ne!(ack.container.unit_name, SERVICE_UNIT);
    assert_eq!(ack.container.launcher_session, [9u8; 16]);
    assert_eq!(ack.limits, limits());
    assert_eq!(ack.isolation, isolation());
    assert_eq!(ack.release.run_id(), run_id);
    assert_eq!(
        ack.release.launcher_session(),
        ack.container.launcher_session
    );
    {
        let script = launcher.script();
        let Some(Request::Launch(frame)) = script.requests.last() else {
            panic!("最后一条应是 launch");
        };
        assert_eq!(frame.argv_digest, argv_digest(&request.argv));
        assert_eq!(frame.nonce, [0xEF; 32]);
        assert_eq!(frame.operation, "inspect_config");
        assert_eq!(frame.mode, None);
        assert_eq!(frame.budget, budget());
        assert_eq!(
            (frame.payload_uid, frame.payload_gid),
            (PAYLOAD_UID, PAYLOAD_GID)
        );
    }

    let container = ack.container.clone();
    adapter.release_once(ack.release).expect("一次放行");
    assert!(launcher.script().released);
    assert_eq!(adapter.observe_running(run_id, &container), Ok(true));
    assert_eq!(
        adapter.drain(run_id, &container),
        Ok(ResourceCounts::default())
    );
    launcher.script().counts = (12, 3, false);
    assert_eq!(
        adapter.drain(run_id, &container),
        Ok(ResourceCounts {
            stdout_bytes: 12,
            stderr_bytes: 3,
            output_over_budget: false,
        })
    );
    assert_eq!(
        adapter.reap(run_id, &container),
        Ok(ExitObservation::NotObserved)
    );
    launcher.script().exit = (true, Some(0), None);
    assert_eq!(
        adapter.reap(run_id, &container),
        Ok(ExitObservation::Code(0))
    );
    launcher.script().exit = (true, None, Some(15));
    assert_eq!(
        adapter.reap(run_id, &container),
        Ok(ExitObservation::Signal(15))
    );
    adapter
        .close_spawn_gate(run_id, &container)
        .expect("入口可关");
    assert!(launcher.script().gated);
    assert_eq!(
        adapter.read_events(run_id, &container),
        Ok(CgroupEvents {
            populated: true,
            frozen: false,
        })
    );
    launcher.script().populated = false;
    assert_eq!(
        adapter.read_events(run_id, &container),
        Ok(CgroupEvents {
            populated: false,
            frozen: false,
        })
    );
    assert_eq!(
        adapter.match_identity(run_id, &container),
        Ok(IdentityMatch::Exact)
    );
    for (identity, expected) in [
        ("unit_missing", IdentityMatch::UnitMissing),
        ("unit_replaced", IdentityMatch::UnitReplaced),
        ("cgroup_reused", IdentityMatch::CgroupReused),
        ("boot_changed", IdentityMatch::BootChanged),
    ] {
        launcher.script().identity = identity;
        assert_eq!(adapter.match_identity(run_id, &container), Ok(expected));
    }
    adapter.stop(run_id, &container, 30).expect("可停止");
    assert_eq!(launcher.script().stops, 1);
    launcher.script().output = Ok(CONFIG_JSON.to_owned());
    assert_eq!(
        adapter.config_output(run_id, &container).as_deref(),
        Ok(CONFIG_JSON)
    );
}

#[test]
fn a_release_is_consumed_once_and_a_foreign_or_repeated_token_never_reaches_the_barrier_twice() {
    let launcher = FakeLauncher::start();
    let approval = FakeApproval::new();
    let mut adapter = adapter(&launcher, &approval);
    // 还没建屏障就放行：本实例没有可绑定的容器，连请求都不发。
    let before = launcher.script().requests.len();
    assert_eq!(
        adapter.release_once(ReleaseToken::new(RunId::from_bytes([1u8; 16]), [9u8; 16])),
        Err(ContainmentGap::IdentityMismatch)
    );
    assert_eq!(launcher.script().requests.len(), before);

    let ack = adapter
        .issue_launch(&launch_request(RunOperation::RunOnce, &approval))
        .expect("屏障可建立");
    let run_id = ack.release.run_id();
    // session 不同的凭证不是本次屏障的：不发请求。
    assert_eq!(
        adapter.release_once(ReleaseToken::new(run_id, [0xEE; 16])),
        Err(ContainmentGap::IdentityMismatch)
    );
    adapter.release_once(ack.release).expect("一次放行");
    // launcher 侧第二次放行是 busy，owner 侧固定映射成 IdentityMismatch。
    assert_eq!(
        adapter.release_once(ReleaseToken::new(run_id, [9u8; 16])),
        Err(ContainmentGap::IdentityMismatch)
    );
    let script = launcher.script();
    let releases = script
        .requests
        .iter()
        .filter(|request| matches!(request, Request::Release(_)))
        .count();
    assert_eq!(releases, 2);
    assert!(script.released);
}

#[test]
fn an_over_budget_output_is_budget_unsupported_and_a_foreign_variant_is_a_protocol_violation() {
    let launcher = FakeLauncher::start();
    let approval = FakeApproval::new();
    let mut adapter = adapter(&launcher, &approval);
    let ack = adapter
        .issue_launch(&launch_request(RunOperation::InspectConfig, &approval))
        .expect("屏障可建立");
    let (run_id, container) = (ack.release.run_id(), ack.container.clone());

    launcher.script().output = Err(Verdict::Budget);
    assert_eq!(
        adapter.config_output(run_id, &container),
        Err(ContainmentGap::BudgetUnsupported)
    );
    // 应答种类与请求不符：不是事实。
    launcher.script().raw = Some(encode(&Response::Running { running: true }));
    assert_eq!(
        adapter.config_output(run_id, &container),
        Err(ContainmentGap::ProtocolViolation)
    );
    launcher.script().raw = Some(encode(&Response::Output {
        output: "x".to_owned(),
    }));
    assert_eq!(adapter.boot_id(), Err(ContainmentGap::ProtocolViolation));
    // reap 应答自相矛盾：既观察到又没有退出码／信号，或两者都给。
    launcher.script().exit = (true, None, None);
    assert_eq!(
        adapter.reap(run_id, &container),
        Err(ContainmentGap::ProtocolViolation)
    );
    launcher.script().exit = (true, Some(1), Some(9));
    assert_eq!(
        adapter.reap(run_id, &container),
        Err(ContainmentGap::ProtocolViolation)
    );
    launcher.script().identity = "weird";
    assert_eq!(
        adapter.match_identity(run_id, &container),
        Err(ContainmentGap::ProtocolViolation)
    );
    // 另一个容器身份或另一个运行标识：launcher 逐字段核对，一律 identity_mismatch。
    let other = ContainerIdentity {
        launcher_session: [0xAA; 16],
        ..container.clone()
    };
    assert_eq!(
        adapter.observe_running(run_id, &other),
        Err(ContainmentGap::IdentityMismatch)
    );
    assert_eq!(
        adapter.observe_running(RunId::from_bytes([0xBB; 16]), &container),
        Err(ContainmentGap::IdentityMismatch)
    );
}

#[test]
fn a_launch_report_that_does_not_echo_the_nonce_or_the_run_id_is_never_a_barrier() {
    let approval = FakeApproval::new();
    for (tamper, gap) in [
        (Tamper::LaunchNonce, ContainmentGap::ProtocolViolation),
        (Tamper::LaunchRunId, ContainmentGap::IdentityMismatch),
    ] {
        let launcher = FakeLauncher::start();
        let mut adapter = adapter(&launcher, &approval);
        launcher.script().tamper = tamper;
        assert_eq!(
            adapter
                .issue_launch(&launch_request(RunOperation::RunOnce, &approval))
                .err(),
            Some(gap),
            "{tamper:?}"
        );
        // 没有登记容器：放行没有对象，也不发请求。
        let before = launcher.script().requests.len();
        assert_eq!(
            adapter.release_once(ReleaseToken::new(RunId::from_bytes([1u8; 16]), [9u8; 16])),
            Err(ContainmentGap::IdentityMismatch)
        );
        assert_eq!(launcher.script().requests.len(), before);
    }
}
