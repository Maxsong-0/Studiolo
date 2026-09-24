//! 线上契约：每种应答编解码往返、`Refused` 的 `Verdict`→`ContainmentGap` 映射、
//! 垫片报告帧往返、启动事实帧往返、超帧应答改回 `Budget`。

use studiolo_sync_launcher::facts::{
    LaunchFacts, LaunchFactsKind, decode_launch_facts, encode_launch_facts, split_report,
};
use studiolo_sync_launcher::protocol::{
    ContainerFrame, Empty, FRAME_MAX_BYTES, IsolationFrame, LaunchReport, LimitsFrame,
    PayloadReport, PayloadReportKind, ProbeReport, Response, Verdict, decode_frame,
    decode_payload_report, encode_payload_report, encode_response, payload_acceptable,
};
use studiolo_sync_transport::linux::ContainmentGap;
use studiolo_sync_transport::owned_run::{
    RunDiagnostics, RunId, RunOperation, RuntimePins, SyncMode,
};

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

fn report() -> PayloadReport {
    PayloadReport {
        kind: PayloadReportKind::PayloadReport,
        mirror_identity: (1, 2),
        config_identity: (1, 3),
        node_identity: (4, 5),
        cli_identity: (4, 6),
        uid: 999,
        gid: 983,
    }
}

fn decode_response(frame: &[u8]) -> Response {
    let payload = decode_frame(frame).expect("帧长一致");
    payload_acceptable(payload).expect("顶层键在闭集内");
    serde_json::from_slice(payload).expect("应答可解")
}

#[test]
fn 每种应答编码后都能过闭集并解回同一值() {
    let responses = [
        Response::Refused {
            verdict: Verdict::Busy,
        },
        Response::Probe(ProbeReport {
            nonce: [5u8; 32],
            protocol_version: 1,
            systemd_version: 259,
            cgroup_v2: true,
            loop_quota: true,
            dedicated_identity: true,
            payload_uid: 999,
            payload_gid: 983,
            boot_id: [2u8; 16],
        }),
        Response::BootId { boot_id: [2u8; 16] },
        Response::Launch(LaunchReport {
            nonce: [5u8; 32],
            container: container(),
            limits: LimitsFrame {
                memory_max: Some(2_147_483_648),
                memory_swap_max: Some(0),
                pids_max: Some(64),
                cpu_max: Some((200_000, 100_000)),
            },
            isolation: IsolationFrame {
                mount_namespace: true,
                pid_namespace: true,
                dedicated_identity: true,
                dropped_capabilities: true,
                payload_cannot_write_cgroup: true,
                final_view_identity_checked: true,
                disk_quota_enforced: false,
            },
            diagnostics: RunDiagnostics {
                pid: Some(4242),
                start_time: Some(5_287_178_087),
            },
        }),
        Response::Release(Empty {}),
        Response::Running { running: true },
        Response::Gate(Empty {}),
        Response::Events {
            populated: false,
            frozen: false,
        },
        Response::Drain {
            stdout_bytes: 12,
            stderr_bytes: 0,
            output_over_budget: false,
        },
        Response::Stop(Empty {}),
        Response::Reap {
            exit_observed: true,
            exit_code: None,
            exit_signal: Some(9),
        },
        Response::Match {
            identity: "cgroup_reused".to_owned(),
        },
        Response::Output {
            output: "{\"syncMode\":\"mirror-remote\"}".to_owned(),
        },
    ];
    for response in responses {
        let frame = encode_response(&response).expect("可编码");
        assert_eq!(decode_response(&frame), response);
    }
}

#[test]
fn 拒绝分类映射到固定缺口() {
    for (verdict, gap) in [
        (Verdict::Protocol, ContainmentGap::ProtocolViolation),
        (Verdict::ArgvMismatch, ContainmentGap::ProtocolViolation),
        (Verdict::Peer, ContainmentGap::PeerRejected),
        (Verdict::IdentityMismatch, ContainmentGap::IdentityMismatch),
        (Verdict::Busy, ContainmentGap::IdentityMismatch),
        (
            Verdict::ManagerUnavailable,
            ContainmentGap::ManagerUnavailable,
        ),
        (Verdict::Budget, ContainmentGap::BudgetUnsupported),
        (
            Verdict::NotProvisioned,
            ContainmentGap::TrustedLauncherNotProvisioned,
        ),
    ] {
        assert_eq!(verdict.gap(), gap, "{verdict:?}");
        let frame = encode_response(&Response::Refused { verdict }).expect("可编码");
        assert_eq!(decode_response(&frame), Response::Refused { verdict });
    }
}

#[test]
fn 垫片报告帧往返且能从stdout头部切出() {
    let frame = encode_payload_report(&report()).expect("可编码");
    assert_eq!(decode_payload_report(&frame).expect("可解"), report());
    let mut stdout = frame.clone();
    stdout.extend_from_slice(b"payload output follows");
    let (parsed, used) = split_report(&stdout).expect("合法").expect("完整");
    assert_eq!((parsed, used), (report(), frame.len()));
    assert_eq!(
        split_report(&frame[..frame.len() - 1]),
        Ok(None),
        "还没写完"
    );
    assert_eq!(split_report(b"ab"), Ok(None), "连长度前缀都没齐");
    assert_eq!(
        split_report(&[0, 0, 0, 0, 1]),
        Err(Verdict::Protocol),
        "长度为零"
    );
    assert_eq!(
        split_report(&[0, 1, 0, 0]),
        Err(Verdict::Protocol),
        "长度超帧上限"
    );
    let mut bad = frame.clone();
    bad[4] = b'[';
    assert_eq!(split_report(&bad), Err(Verdict::Protocol), "形状不对");
}

#[test]
fn 启动事实帧往返() {
    let facts = LaunchFacts {
        kind: LaunchFactsKind::LaunchFacts,
        runtime: RuntimePins {
            version: "0.0.14".to_owned(),
            node_path: "/opt/studiolo/node/bin/node".into(),
            cli_path: "/opt/studiolo/obsidian-headless/cli.js".into(),
            closure_digest: [7u8; 32],
        },
        mirror_root: "/var/lib/studiolo-sync/mirror".into(),
        payload_config_root: "/var/lib/studiolo-sync/config".into(),
        operation: RunOperation::SetMode(SyncMode::MirrorRemote),
    };
    let frame = encode_launch_facts(&facts).expect("可编码");
    assert_eq!(decode_launch_facts(&frame).expect("可解"), facts);
    assert_eq!(decode_launch_facts(b"\0\0\0\x02{}"), Err(Verdict::Protocol));
}

#[test]
fn 超帧的应答改回预算拒绝() {
    let output = Response::Output {
        output: "x".repeat(FRAME_MAX_BYTES),
    };
    assert_eq!(encode_response(&output), Err(Verdict::Budget));
}
