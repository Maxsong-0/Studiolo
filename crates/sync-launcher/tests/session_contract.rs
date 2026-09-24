//! session 状态机契约（纯逻辑）：放行只一次、关门后拒绝、reap 前必须关门、replaceable 语义、身份逐字段核对。

use studiolo_sync_launcher::session::{Phase, Session};
use studiolo_sync_transport::linux::protocol::{ContainerFrame, Verdict};
use studiolo_sync_transport::owned_run::{RunBudget, RunId};

fn container() -> ContainerFrame {
    ContainerFrame {
        run_id: RunId::from_bytes([1u8; 16]),
        boot_id: [2u8; 16],
        unit_name: "studiolo-sync-payload-7-01010101.service".to_owned(),
        invocation_id: [3u8; 16],
        payload_cgroup: "/sys/fs/cgroup/studiolo-sync.slice/x.service".to_owned(),
        cgroup_identity: (64, 99),
        launcher_session: [4u8; 16],
    }
}

fn budget() -> RunBudget {
    RunBudget {
        deadline_seconds: 3600,
        stop_grace_seconds: 30,
        stdout_bytes: 262_144,
        stderr_bytes: 262_144,
        config_json_bytes: 65_536,
        private_state_bytes: 1 << 26,
        log_bytes: 1 << 28,
        memory_bytes: 1 << 31,
        pids_max: 64,
        cpu_max_percent: 200,
    }
}

#[test]
fn 放行只被接受一次() {
    let mut session = Session::barrier(container(), budget());
    assert_eq!(session.phase, Phase::Barrier);
    assert_eq!(session.release(), Ok(()));
    assert_eq!(session.phase, Phase::Released);
    assert_eq!(session.release(), Err(Verdict::Busy));
    assert_eq!(session.phase, Phase::Released);
}

#[test]
fn 关门后再也不放行() {
    let mut session = Session::barrier(container(), budget());
    session.gate();
    assert_eq!(session.phase, Phase::Gated);
    assert_eq!(session.release(), Err(Verdict::Busy));
    session.gate();
    assert_eq!(session.phase, Phase::Gated, "关门幂等");
    let mut released = Session::barrier(container(), budget());
    released.release().expect("首次放行");
    released.gate();
    assert_eq!(released.phase, Phase::Gated, "放行后也能关门");
    assert_eq!(released.release(), Err(Verdict::Busy));
}

#[test]
fn 未关门不能reap_关门后reap只登记一次() {
    let mut session = Session::barrier(container(), budget());
    assert_eq!(session.reap(Some(0), None), Err(Verdict::Busy));
    session.release().expect("放行");
    assert_eq!(
        session.reap(Some(0), None),
        Err(Verdict::Busy),
        "放行后未关门仍不能 reap"
    );
    session.gate();
    assert_eq!(session.reap(Some(7), None), Ok(()));
    assert_eq!(session.phase, Phase::Reaped);
    assert_eq!((session.exit_code, session.exit_signal), (Some(7), None));
    assert_eq!(
        session.reap(None, Some(9)),
        Err(Verdict::Busy),
        "已 reap 不再改写"
    );
    assert_eq!((session.exit_code, session.exit_signal), (Some(7), None));
    session.gate();
    assert_eq!(session.phase, Phase::Reaped, "reap 后关门不改阶段");
}

#[test]
fn 只有reap或退休后才可被替换() {
    let mut session = Session::barrier(container(), budget());
    assert!(!session.replaceable());
    session.release().expect("放行");
    assert!(!session.replaceable());
    session.gate();
    assert!(!session.replaceable());
    session.reap(None, Some(15)).expect("reap");
    assert!(session.replaceable());
    session.phase = Phase::Retired;
    assert!(session.replaceable());
}

#[test]
fn 请求帧身份逐字段核对() {
    let session = Session::barrier(container(), budget());
    assert_eq!(session.check(&container()), Ok(()));
    let mut wrong = container();
    wrong.run_id = RunId::from_bytes([9u8; 16]);
    assert_eq!(session.check(&wrong), Err(Verdict::IdentityMismatch));
    let mut wrong = container();
    wrong.boot_id = [9u8; 16];
    assert_eq!(session.check(&wrong), Err(Verdict::IdentityMismatch));
    let mut wrong = container();
    wrong.unit_name.push('x');
    assert_eq!(session.check(&wrong), Err(Verdict::IdentityMismatch));
    let mut wrong = container();
    wrong.invocation_id = [9u8; 16];
    assert_eq!(session.check(&wrong), Err(Verdict::IdentityMismatch));
    let mut wrong = container();
    wrong.payload_cgroup.push('x');
    assert_eq!(session.check(&wrong), Err(Verdict::IdentityMismatch));
    let mut wrong = container();
    wrong.cgroup_identity = (64, 100);
    assert_eq!(session.check(&wrong), Err(Verdict::IdentityMismatch));
    let mut wrong = container();
    wrong.launcher_session = [9u8; 16];
    assert_eq!(session.check(&wrong), Err(Verdict::IdentityMismatch));
}
