//! Linux adapter 的纯逻辑契约：解析、比对与封闭 argv，全部不触碰真实 OS。

use crate::linux::cgroup::{
    CgroupLimits, parse_cpu_max, parse_events, parse_scalar, satisfies, within_prefix,
};
use crate::linux::ipc::{
    FRAME_MAX_BYTES, PeerCredentials, decode_frame, encode_frame, payload_acceptable,
    peer_acceptable,
};
use crate::linux::launcher::{HandshakeNonce, IsolationReport, isolation_sufficient};
use crate::linux::service::{
    ServiceProbe, build_argv, environment_acceptable, is_quiescence_evidence, same_invocation,
    unit_name_acceptable,
};
use crate::linux::stdio::{DrainVerdict, OutputDrain, Stream, config_output_acceptable};
use crate::owned_run::RunOperation;

use super::support::*;

#[test]
fn cgroup_events_are_parsed_strictly_and_an_unpopulated_group_is_the_only_empty_proof() {
    let events = parse_events("populated 0\nfrozen 0\n").expect("标准格式可解析");
    assert!(!events.populated);
    assert!(!events.frozen);
    assert!(
        parse_events("populated 1\n")
            .expect("缺 frozen 也可解析")
            .populated
    );

    for bad in [
        "frozen 0\n",
        "populated 2\n",
        "populated\n",
        "populated 0\npopulated 0\n",
        "populated 0\nunknown 1\n",
        "",
    ] {
        assert!(parse_events(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_service_state_is_never_accepted_as_a_quiescence_proof() {
    let probe = ServiceProbe {
        unit_name: SERVICE_UNIT.to_owned(),
        active_state: "inactive".to_owned(),
        main_pid: 0,
        invocation_id: [8u8; 16],
    };
    // `MainPID=0` 与 `ActiveState=inactive` 都不是空组证明。
    assert!(!is_quiescence_evidence(&probe));
    assert!(same_invocation(&probe, SERVICE_UNIT, [8u8; 16]));
    assert!(!same_invocation(&probe, SERVICE_UNIT, [9u8; 16]));
    assert!(!same_invocation(&probe, "other.service", [8u8; 16]));
}

#[test]
fn actual_limits_must_be_read_back_and_never_left_unlimited() {
    assert_eq!(parse_scalar("max").expect("可解析"), None);
    assert_eq!(parse_scalar(" 4096\n").expect("可解析"), Some(4096));
    assert!(parse_scalar("-1").is_err());
    assert_eq!(
        parse_cpu_max("200000 100000").expect("可解析"),
        Some((200_000, 100_000))
    );
    assert_eq!(parse_cpu_max("max 100000").expect("可解析"), None);
    assert!(parse_cpu_max("200000").is_err());
    assert!(parse_cpu_max("200000 0").is_err());
    assert!(parse_cpu_max("200000 100000 1").is_err());

    let budget = budget();
    assert!(satisfies(&limits(), &budget).is_ok());
    for broken in [
        CgroupLimits {
            memory_max: None,
            ..limits()
        },
        CgroupLimits {
            memory_swap_max: Some(1),
            ..limits()
        },
        CgroupLimits {
            memory_swap_max: None,
            ..limits()
        },
        CgroupLimits {
            pids_max: Some(65),
            ..limits()
        },
        CgroupLimits {
            cpu_max: Some((300_000, 100_000)),
            ..limits()
        },
        CgroupLimits::default(),
    ] {
        assert!(satisfies(&broken, &budget).is_err(), "{broken:?}");
    }
}

#[test]
fn a_payload_subgroup_must_sit_strictly_inside_the_approved_prefix() {
    assert!(within_prefix(
        &format!("{CGROUP_PREFIX}/run-1"),
        CGROUP_PREFIX
    ));
    for bad in [
        CGROUP_PREFIX,
        "/sys/fs/cgroup/studiolo.slice/payload-other/run",
        "/sys/fs/cgroup/other/run",
        "relative/run",
    ] {
        assert!(!within_prefix(bad, CGROUP_PREFIX), "{bad}");
    }
}

#[test]
fn the_closed_argv_never_contains_a_shell_a_path_search_or_a_continuous_flag() {
    let approval = FakeApproval::new();
    let mirror = std::path::Path::new(MIRROR_ROOT);

    let once = build_argv(&approval.runtime, RunOperation::RunOnce, mirror).expect("可以构造");
    assert_eq!(
        once,
        vec![
            "/opt/studiolo/node/bin/node".to_owned(),
            "/opt/studiolo/obsidian-headless/cli.js".to_owned(),
            "sync".to_owned(),
            "--path".to_owned(),
            MIRROR_ROOT.to_owned(),
        ]
    );
    let inspect =
        build_argv(&approval.runtime, RunOperation::InspectConfig, mirror).expect("可以构造");
    assert_eq!(inspect.last().expect("末位参数"), "--json");
    assert!(inspect.contains(&"sync-config".to_owned()));
    for argv in [&once, &inspect] {
        assert!(!argv.iter().any(|item| item.contains("--continuous")));
        assert!(!argv.iter().any(|item| item.contains("sh -c")));
        assert!(argv[0].starts_with('/'));
    }
    assert!(
        build_argv(
            &approval.runtime,
            RunOperation::RunOnce,
            std::path::Path::new("mirror")
        )
        .is_err()
    );

    assert!(environment_acceptable(&["LANG", "TZ"]));
    for name in [
        "OBSIDIAN_AUTH_TOKEN",
        "NODE_OPTIONS",
        "NODE_PATH",
        "HOME",
        "",
    ] {
        assert!(!environment_acceptable(&[name]), "{name}");
    }
    assert!(unit_name_acceptable(SERVICE_UNIT));
    assert!(!unit_name_acceptable("bad unit.service"));
    assert!(!unit_name_acceptable(".service"));
}

#[test]
fn launcher_frames_are_bounded_and_carry_only_closed_fields() {
    let payload = br#"{"run_id":"00","nonce":"01"}"#;
    let frame = encode_frame(payload).expect("可以编码");
    assert_eq!(decode_frame(&frame).expect("可以解码"), payload);

    assert!(encode_frame(&vec![b'x'; FRAME_MAX_BYTES + 1]).is_err());
    assert!(encode_frame(b"").is_err());
    assert!(decode_frame(b"\0\0\0").is_err());
    assert!(decode_frame(b"\0\0\0\x05ab").is_err());
    assert!(decode_frame(&[255, 255, 255, 255]).is_err());

    assert!(payload_acceptable(payload).is_ok());
    for bad in [
        br#"{"auth_token":"x"}"#.as_slice(),
        br#"{"argv":["/bin/sh"]}"#.as_slice(),
        br#"{"mirror_path":"/srv"}"#.as_slice(),
        b"{}".as_slice(),
        b"[1]".as_slice(),
        b"not json".as_slice(),
    ] {
        assert!(payload_acceptable(bad).is_err(), "{:?}", bad);
    }

    let peer = PeerCredentials {
        uid: 61001,
        gid: 61001,
        pid: 4242,
    };
    assert!(peer_acceptable(&peer, 61001, 61001).is_ok());
    assert!(peer_acceptable(&peer, 1000, 61001).is_err());
    assert!(
        peer_acceptable(
            &PeerCredentials {
                uid: 0,
                gid: 0,
                pid: 1
            },
            0,
            0
        )
        .is_err()
    );
}

#[test]
fn the_handshake_nonce_never_appears_in_any_printable_output() {
    let nonce = HandshakeNonce::from_bytes([0xEF; 32]);
    let rendered = format!("{nonce:?}");
    assert_eq!(rendered, "HandshakeNonce(<redacted>)");
    assert!(!rendered.contains("ef"));
    assert!(nonce.matches(&HandshakeNonce::from_bytes([0xEF; 32])));
    assert!(!nonce.matches(&HandshakeNonce::from_bytes([0x00; 32])));
}

#[test]
fn any_missing_isolation_check_forbids_opening_the_barrier() {
    assert!(isolation_sufficient(&isolation()).is_ok());
    let cases: [fn(&mut IsolationReport); 7] = [
        |report| report.mount_namespace = false,
        |report| report.pid_namespace = false,
        |report| report.final_view_identity_checked = false,
        |report| report.dedicated_identity = false,
        |report| report.dropped_capabilities = false,
        |report| report.payload_cannot_write_cgroup = false,
        |report| report.disk_quota_enforced = false,
    ];
    for mutate in cases {
        let mut report = isolation();
        mutate(&mut report);
        assert!(isolation_sufficient(&report).is_err());
    }
    assert!(isolation_sufficient(&IsolationReport::default()).is_err());
}

#[test]
fn output_over_the_limit_stops_the_run_and_permanently_forbids_parsing() {
    let budget = budget();
    let mut drain = OutputDrain::default();
    assert_eq!(
        drain.absorb(Stream::Stdout, 1024, &budget),
        DrainVerdict::Continue
    );
    assert!(drain.parse_allowed());
    assert!(config_output_acceptable(&drain, &budget).is_ok());

    assert_eq!(
        drain.absorb(Stream::Stdout, u64::from(budget.stdout_bytes), &budget),
        DrainVerdict::OverBudget
    );
    // 超限过就永远不允许解析，绝不截断后按成功处理。
    assert!(!drain.parse_allowed());
    assert!(config_output_acceptable(&drain, &budget).is_err());
    assert!(drain.counts().output_over_budget);

    // 计数溢出按超限处理，绝不回绕成「仍在预算内」。
    let mut overflow = OutputDrain::default();
    overflow.absorb(Stream::Stderr, u64::MAX, &budget);
    assert_eq!(
        overflow.absorb(Stream::Stderr, 1, &budget),
        DrainVerdict::OverBudget
    );
    assert!(!overflow.parse_allowed());
}
