//! 集成测试夹具：探测用户 manager、临时根与假 runtime、socket 客户端、轮询。

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use studiolo_sync_launcher::config::LauncherConfig;
use studiolo_sync_launcher::protocol::{
    ContainerFrame, LaunchFrame, Request, Response, decode_frame, encode_frame,
};
use studiolo_sync_launcher::serve::Layout;
use studiolo_sync_transport::owned_run::{RunBudget, RunId, RuntimePins};

/// 可用的用户 manager：`XDG_RUNTIME_DIR` 在且 `systemd-run --user` 能跑完一个 unit；返回运行时目录。
pub fn user_manager() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    let ok = Command::new("/usr/bin/systemd-run")
        .args(["--user", "--quiet", "--collect", "--wait", "/bin/true"])
        .status()
        .is_ok_and(|status| status.success());
    ok.then_some(runtime)
}

pub fn user_show(unit: &str, property: &str) -> String {
    let output = Command::new("/usr/bin/systemctl")
        .args(["--user", "show", "--value", "-p", property, "--", unit])
        .output()
        .expect("systemctl --user show");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// 一次连接一帧请求一帧应答，与 W1 的 client 同一线上形状。
pub fn call(socket: &Path, request: &Request) -> Response {
    let mut stream = UnixStream::connect(socket).expect("连上 launcher socket");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let frame = encode_frame(&serde_json::to_vec(request).unwrap()).unwrap();
    stream.write_all(&frame).unwrap();
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).expect("应答帧头");
    let length = u32::from_be_bytes(header) as usize;
    let mut whole = header.to_vec();
    whole.resize(4 + length, 0);
    stream.read_exact(&mut whole[4..]).expect("应答帧体");
    serde_json::from_slice(decode_frame(&whole).unwrap()).expect("应答可解")
}

pub fn budget() -> RunBudget {
    RunBudget {
        deadline_seconds: 600,
        stop_grace_seconds: 1,
        stdout_bytes: 65_536,
        stderr_bytes: 65_536,
        config_json_bytes: 65_536,
        private_state_bytes: 1 << 20,
        log_bytes: 1 << 20,
        memory_bytes: 268_435_456,
        pids_max: 64,
        cpu_max_percent: 150,
    }
}

fn random16() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("随机字节");
    bytes
}

/// 每次新 run_id：unit 名带 run_id 前 8 hex，旧的失败残留不会撞名。
pub fn launch_frame(config: &LauncherConfig, sequence: u64, digest: [u8; 32]) -> LaunchFrame {
    LaunchFrame {
        run_id: RunId::from_bytes(random16()),
        sequence,
        operation: "run_once".to_owned(),
        mode: None,
        nonce: [7u8; 32],
        argv_digest: digest,
        budget: budget(),
        payload_uid: config.payload_uid,
        payload_gid: config.payload_gid,
    }
}

pub fn wait_unpopulated(socket: &Path, container: &ContainerFrame) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match call(socket, &Request::Events(container.clone())) {
            Response::Events {
                populated: false, ..
            } => return,
            Response::Events { .. } => {}
            other => panic!("events 应答异常：{other:?}"),
        }
        assert!(Instant::now() < deadline, "payload 没有在 20 s 内退出");
        thread::sleep(Duration::from_millis(100));
    }
}

pub struct Fixture {
    _temp: tempfile::TempDir,
    pub config: LauncherConfig,
    pub layout: Layout,
    pub runtime: RuntimePins,
    pub mirror: PathBuf,
    pub config_root: PathBuf,
}

/// 临时根放在 `XDG_RUNTIME_DIR` 下：它不受 `PrivateTmp=` 影响，`ReadWritePaths=` 能把镜像根与配置根挂进只读视图。
/// `/bin/sh` 冒充 node，`cli.sh` 冒充 cli.js：垫片 exec 的封闭 argv 就是 `/bin/sh cli.sh sync --path <镜像根>`。
pub fn fixture(runtime_dir: &Path) -> Fixture {
    let temp = tempfile::Builder::new()
        .prefix("studiolo-launcher-")
        .tempdir_in(runtime_dir)
        .expect("临时根");
    let root = temp.path().to_path_buf();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let loop_mount = root.join("loop");
    let mirror = loop_mount.join("mirror");
    let config_root = loop_mount.join("config");
    fs::create_dir_all(&mirror).unwrap();
    fs::create_dir_all(&config_root).unwrap();
    fs::write(root.join("loop.img"), b"").unwrap();
    let cli = root.join("cli.sh");
    fs::write(
        &cli,
        "printf 'cli says: %s\\n' \"$*\"\nprintf 'xdg=%s lang=%s tz=%s home=%s\\n' \"$XDG_CONFIG_HOME\" \"$LANG\" \"$TZ\" \"${HOME:-unset}\"\n",
    )
    .unwrap();
    let manager_root = user_show("-.slice", "ControlGroup");
    assert!(
        manager_root.starts_with('/'),
        "用户 manager 的根组：{manager_root:?}"
    );
    let uid = rustix::process::getuid().as_raw();
    let gid = rustix::process::getgid().as_raw();
    let runtime = RuntimePins {
        version: "0.0.14".to_owned(),
        node_path: PathBuf::from("/bin/sh"),
        cli_path: cli,
        closure_digest: [0u8; 32],
    };
    let document = serde_json::json!({
        "schema_version": 1,
        "owner_uid": uid,
        "owner_gid": gid,
        "payload_uid": uid + 1,
        "payload_gid": gid + 1,
        "runtime": runtime,
        "mirror_root": mirror,
        "payload_config_root": config_root,
        "loop_image": root.join("loop.img"),
        "loop_mount": loop_mount,
        "inaccessible": ["/root"],
        // slice 名的连字符是层级：studiolo-launcher-test.slice 住在 studiolo.slice/studiolo-launcher.slice 之下。
        "payload_cgroup_prefix": format!(
            "/sys/fs/cgroup{manager_root}/studiolo.slice/studiolo-launcher.slice/studiolo-launcher-test.slice"
        ),
    });
    let config =
        LauncherConfig::parse(&serde_json::to_vec(&document).unwrap()).expect("测试配置合法");
    let layout = Layout {
        socket_dir: root.join("run"),
        socket: root.join("run/launcher.sock"),
        sessions: root.join("run/sessions"),
    };
    Fixture {
        _temp: temp,
        config,
        layout,
        runtime,
        mirror,
        config_root,
    }
}

/// Stop 后保留真实等待结果与静止事实，不提前清掉 owner 恢复所需的身份。
pub fn assert_retained(socket: &Path, container: &ContainerFrame, code: i32) {
    assert_eq!(user_show(&container.unit_name, "LoadState"), "loaded");
    assert_eq!(
        call(socket, &Request::Match(container.clone())),
        Response::Match {
            identity: "exact".to_owned()
        }
    );
    assert_eq!(
        call(socket, &Request::Reap(container.clone())),
        Response::Reap {
            exit_observed: true,
            exit_code: Some(code),
            exit_signal: None
        }
    );
    assert_eq!(
        call(socket, &Request::Events(container.clone())),
        Response::Events {
            populated: false,
            frozen: false
        }
    );
}

/// 仅清理本测试明确持有的随机 unit；这是夹具回收，不是产品 Stop 语义。
pub fn cleanup_unit(unit: &str) {
    assert!(
        Command::new("/usr/bin/systemctl")
            .args(["--user", "stop", "--", unit])
            .status()
            .unwrap()
            .success()
    );
    let _ = Command::new("/usr/bin/systemctl")
        .args(["--user", "reset-failed", "--", unit])
        .status();
}
