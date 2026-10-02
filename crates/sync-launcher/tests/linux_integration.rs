//! 真实 `systemd --user` 上的 launch→release→reap→events 集成回路，走与生产同一条 `serve_with` 路径。
//! 默认 `#[ignore = "平台契约未验"]`：`cargo test` 只显示 ignored；在有可用用户管理器的机器上用
//! `cargo test -p studiolo-sync-launcher --test linux_integration -- --include-ignored` 真跑。
//! 强制运行而机器不具备条件时直接失败并打印「平台契约未验」，绝不 skip 冒充绿。
//! 用户 manager 起不了 PID 视图、清不了 capability、也不能切换身份，这几项在应答里如实为 false。
#![cfg(target_os = "linux")]

#[path = "linux_integration/stop.rs"]
mod stop;
#[path = "linux_integration/support.rs"]
mod support;

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};
use studiolo_sync_launcher::facts;
use studiolo_sync_launcher::protocol::{Empty, Request, Response, StopFrame, Verdict};
use studiolo_sync_launcher::serve::serve_with;
use studiolo_sync_launcher::unit::Manager;
use studiolo_sync_transport::linux::service;
use studiolo_sync_transport::owned_run::RunOperation;
use support::{
    assert_retained, call, cleanup_unit, fixture, launch_frame, user_manager, user_show,
    wait_unpopulated,
};

const UNVERIFIED: &str = "平台契约未验：本机没有可用的 systemd --user，launcher 集成测试未跑";
const PAYLOAD: &str = env!("CARGO_BIN_EXE_studiolo-sync-launcher");

#[test]
#[ignore = "平台契约未验：需要可用的 systemd --user；用 --include-ignored 真跑"]
fn 真实用户manager上的launch_release_reap_events回路() {
    let Some(runtime_dir) = user_manager() else {
        panic!("{UNVERIFIED}");
    };
    let fixture = fixture(&runtime_dir);
    let (config, layout) = (fixture.config.clone(), fixture.layout.clone());
    thread::spawn(move || {
        serve_with(config, Manager::user(PathBuf::from(PAYLOAD)), &layout).expect("serve_with");
    });
    let socket = fixture.layout.socket.clone();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !socket.exists() {
        assert!(Instant::now() < deadline, "socket 没出现");
        thread::sleep(Duration::from_millis(20));
    }
    let config = &fixture.config;

    // probe 与 boot_id 都是实测。
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    let boot_id = facts::parse_hex16(&boot).unwrap();
    match call(&socket, &Request::Probe { nonce: [9u8; 32] }) {
        Response::Probe(report) => {
            assert_eq!(report.nonce, [9u8; 32]);
            assert_eq!(report.protocol_version, 1);
            assert!(report.systemd_version >= facts::MIN_SYSTEMD_VERSION);
            assert!(report.cgroup_v2);
            assert!(!report.loop_quota, "临时目录不在 loop 镜像上");
            assert_eq!(
                (report.payload_uid, report.payload_gid),
                (config.payload_uid, config.payload_gid)
            );
            assert_eq!(report.boot_id, boot_id);
        }
        other => panic!("probe 应答异常：{other:?}"),
    }
    assert_eq!(
        call(&socket, &Request::BootId(Empty {})),
        Response::BootId { boot_id }
    );

    // argv 摘要不符先于一切建 unit 的动作被拒。
    let argv =
        service::build_argv(&fixture.runtime, RunOperation::RunOnce, &fixture.mirror).unwrap();
    let digest = service::argv_digest(&argv);
    let mut wrong = argv.clone();
    wrong.push("--continuous".to_owned());
    assert_eq!(
        call(
            &socket,
            &Request::Launch(launch_frame(config, 1, service::argv_digest(&wrong)))
        ),
        Response::Refused {
            verdict: Verdict::ArgvMismatch
        }
    );

    // 第一轮：launch → busy 拒第二个 → running=false → release（只一次）→ 退出 → drain/gate/reap/output/match → stop 保留等待证据。
    let first = launch_frame(config, 1, digest);
    let report = match call(&socket, &Request::Launch(first.clone())) {
        Response::Launch(report) => report,
        other => panic!("launch 应答异常：{other:?}"),
    };
    let container = report.container.clone();
    assert_eq!(report.nonce, first.nonce);
    assert_eq!(container.unit_name, facts::unit_name(1, &first.run_id));
    assert!(
        container
            .payload_cgroup
            .starts_with(config.payload_cgroup_prefix.to_str().unwrap())
    );
    assert_eq!(container.boot_id, boot_id);
    assert!(report.diagnostics.pid.is_some_and(|pid| pid > 0));
    assert_eq!(report.limits.memory_max, Some(268_435_456));
    assert_eq!(report.limits.memory_swap_max, Some(0));
    assert_eq!(report.limits.pids_max, Some(64));
    assert_eq!(report.limits.cpu_max, Some((150_000, 100_000)));
    let isolation = report.isolation;
    assert!(
        isolation.final_view_identity_checked,
        "垫片在最终视图里 stat 到的四个对象与 launcher 一致"
    );
    assert!(isolation.payload_cannot_write_cgroup);
    assert!(!isolation.pid_namespace, "用户 manager 起不了 PID 视图");
    assert!(
        !isolation.dropped_capabilities,
        "用户 manager 清不了 capability"
    );
    assert!(!isolation.dedicated_identity, "用户 manager 不能切换身份");
    assert!(!isolation.mount_namespace, "用户 manager 只能保留只读 HOME");
    assert!(!isolation.disk_quota_enforced);
    assert_eq!(
        call(&socket, &Request::Launch(launch_frame(config, 2, digest))),
        Response::Refused {
            verdict: Verdict::Busy
        }
    );
    assert_eq!(
        call(&socket, &Request::Running(container.clone())),
        Response::Running { running: false }
    );
    match call(&socket, &Request::Events(container.clone())) {
        Response::Events { populated, .. } => assert!(populated, "垫片正在屏障前等待"),
        other => panic!("events 应答异常：{other:?}"),
    }
    assert_eq!(
        call(&socket, &Request::Release(container.clone())),
        Response::Release(Empty {})
    );
    assert_eq!(
        call(&socket, &Request::Release(container.clone())),
        Response::Refused {
            verdict: Verdict::Busy
        },
        "放行只消费一次"
    );
    wait_unpopulated(&socket, &container);
    assert_eq!(
        call(&socket, &Request::Running(container.clone())),
        Response::Running { running: false }
    );
    let expected_output = format!(
        "cli says: sync --path {}\nxdg={} lang=C.UTF-8 tz=UTC home=unset\n",
        fixture.mirror.display(),
        fixture.config_root.display()
    );
    assert_eq!(
        call(&socket, &Request::Drain(container.clone())),
        Response::Drain {
            stdout_bytes: expected_output.len() as u64,
            stderr_bytes: 0,
            output_over_budget: false,
        }
    );
    assert_eq!(
        call(&socket, &Request::Reap(container.clone())),
        Response::Refused {
            verdict: Verdict::Busy
        },
        "未关门不能 reap"
    );
    assert_eq!(
        call(&socket, &Request::Gate(container.clone())),
        Response::Gate(Empty {})
    );
    assert_eq!(
        call(&socket, &Request::Reap(container.clone())),
        Response::Reap {
            exit_observed: true,
            exit_code: Some(0),
            exit_signal: None
        }
    );
    assert_eq!(
        call(&socket, &Request::Output(container.clone())),
        Response::Output {
            output: expected_output.clone()
        }
    );
    assert_eq!(
        call(&socket, &Request::Match(container.clone())),
        Response::Match {
            identity: "exact".to_owned()
        }
    );
    assert_eq!(
        call(
            &socket,
            &Request::Stop(StopFrame {
                container: container.clone(),
                grace_seconds: 1
            })
        ),
        Response::Stop(Empty {})
    );
    assert_retained(&socket, &container, 0);
    assert_eq!(
        call(&socket, &Request::Output(container.clone())),
        Response::Output {
            output: expected_output
        }
    );
    assert_eq!(fs::read_dir(&fixture.layout.sessions).unwrap().count(), 1);
    let old = container;

    // 第二轮：不放行直接关门，垫片读到 EOF 退出 4。
    let second = launch_frame(config, 2, digest);
    let container = match call(&socket, &Request::Launch(second)) {
        Response::Launch(report) => report.container,
        other => panic!("第二轮 launch 应答异常：{other:?}"),
    };
    assert_eq!(user_show(&old.unit_name, "LoadState"), "not-found");
    assert_eq!(
        call(&socket, &Request::Events(old)),
        Response::Refused {
            verdict: Verdict::IdentityMismatch
        }
    );
    assert_eq!(fs::read_dir(&fixture.layout.sessions).unwrap().count(), 1);
    assert_eq!(
        call(&socket, &Request::Gate(container.clone())),
        Response::Gate(Empty {})
    );
    assert_eq!(
        call(&socket, &Request::Release(container.clone())),
        Response::Refused {
            verdict: Verdict::Busy
        },
        "关门后永不放行"
    );
    wait_unpopulated(&socket, &container);
    assert_eq!(
        call(&socket, &Request::Reap(container.clone())),
        Response::Reap {
            exit_observed: true,
            exit_code: Some(4),
            exit_signal: None
        }
    );
    assert_eq!(
        call(
            &socket,
            &Request::Stop(StopFrame {
                container: container.clone(),
                grace_seconds: 1
            })
        ),
        Response::Stop(Empty {})
    );
    assert_retained(&socket, &container, 4);
    cleanup_unit(&container.unit_name);
}
