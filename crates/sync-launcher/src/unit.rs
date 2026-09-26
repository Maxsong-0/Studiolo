//! systemd 的真实调用（守卫按精确记号登记）：`systemd-run` 与 `systemctl` 只用固定绝对路径、
//! 封闭 argv、`env_clear`，子进程有界读取并有超时。cgroupfs／proc 读取在 `unit/reads.rs`，
//! session 目录与控制 FIFO 在 `unit/session.rs`；纯判定在 `facts.rs`，编排在 `handler.rs`。

mod reads;
mod session;

pub use reads::{cgroup_dir, cgroup_v2, events, limits, main_exe, read_bounded, read_text, stat};
pub use session::{ReleaseGate, SessionDir};

use crate::config::{self, LauncherConfig};
use crate::facts;
use crate::protocol::{LaunchFrame, Verdict};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// systemd 属性参数的固定前缀；每个属性都写成 `--property=KEY=VALUE`。
const PROPERTY: &str = "--property=";
const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(5);
/// `Type=exec` 下 `systemd-run` 等到 exec 成功才返回；设计上限 10 s。
const SYSTEMD_RUN_TIMEOUT: Duration = Duration::from_secs(10);
/// 子进程 stdout 读取上限（`systemctl show` 的十几个属性远小于它）。
const OUTPUT_CAP: usize = 64 * 1024;
/// 只有系统 manager 能做到的属性：用户 manager 起不了 PID 视图、清不了 capability、建不了私有 /dev，
/// 也不能切换身份；集成测试只验流程，这几项在应答里如实为 false。
const SYSTEM_ONLY: [&str; 4] = [
    "ProtectHome=yes",
    "PrivatePIDs=yes",
    "PrivateDevices=yes",
    "CapabilityBoundingSet=",
];

/// 拒绝前把固定原因写到 stderr（生产进 journal）；只有步骤名与分类，没有路径正文以外的任何秘密。
pub fn refused(step: &str, verdict: Verdict) -> Verdict {
    eprintln!("launcher 拒绝：{step} → {verdict:?}");
    verdict
}

/// 用哪个 manager 起 transient unit：生产固定系统 manager 与本程序的固定安装路径；
/// 集成测试用 `--user` 与指定的垫片可执行文件。
#[derive(Clone, Debug)]
pub struct Manager {
    user: bool,
    payload_binary: PathBuf,
}

impl Manager {
    /// 生产：系统 manager，垫片就是 `config::LAUNCHER_BINARY`。
    #[must_use]
    pub fn system() -> Self {
        Self {
            user: false,
            payload_binary: PathBuf::from(config::LAUNCHER_BINARY),
        }
    }

    /// 集成测试：`systemd-run --user`，垫片是指定的可执行文件。
    #[must_use]
    pub fn user(payload_binary: PathBuf) -> Self {
        Self {
            user: true,
            payload_binary,
        }
    }

    /// 是否用户 manager。
    #[must_use]
    pub fn is_user(&self) -> bool {
        self.user
    }

    fn environment(&self, command: &mut Command) {
        command.env_clear();
        if self.user {
            // 用户 manager 只能经会话总线找到；除这两个变量外不继承任何环境。
            for name in ["XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
        }
    }

    /// `systemctl --version` 的主版本。
    ///
    /// # Errors
    /// manager 不可用或输出不可解析返回 `ManagerUnavailable`。
    pub fn version(&self) -> Result<u32, Verdict> {
        let output = self.systemctl(&["--version"])?;
        facts::systemd_version(&String::from_utf8_lossy(&output)).ok_or(Verdict::ManagerUnavailable)
    }

    /// 起 transient service：垫片停在控制 FIFO 的放行字节前；`systemd-run` 等到 exec 成功才返回。
    ///
    /// # Errors
    /// 路径不能进属性值返回 `NotProvisioned`；`systemd-run` 失败或超时返回 `ManagerUnavailable`。
    pub fn run_transient(
        &self,
        config: &LauncherConfig,
        unit: &str,
        dir: &SessionDir,
        frame: &LaunchFrame,
    ) -> Result<(), Verdict> {
        let acceptable = [&config.mirror_root, &config.payload_config_root]
            .into_iter()
            .chain(config.inaccessible.iter())
            .all(|path| facts::property_path_acceptable(path));
        if !acceptable {
            return Err(refused(
                "配置路径含空白，进不了 systemd 属性",
                Verdict::NotProvisioned,
            ));
        }
        let mut command = Command::new(config::SYSTEMD_RUN);
        self.environment(&mut command);
        if self.user {
            command.arg("--user");
        } else {
            command
                .arg(format!("--uid={}", config.payload_uid))
                .arg(format!("--gid={}", config.payload_gid));
        }
        command
            .arg(format!("--unit={unit}"))
            .args(["--service-type=exec", "--quiet"]);
        let Some(slice) = facts::slice_from_prefix(&config.payload_cgroup_prefix) else {
            return Err(refused(
                "批准前缀不是 slice 的完整层级展开",
                Verdict::NotProvisioned,
            ));
        };
        command.arg(format!("--slice={slice}"));
        for property in properties(config, dir, frame, self.user) {
            command.arg(format!("{PROPERTY}{property}"));
        }
        command.arg("--").arg(&self.payload_binary).arg("payload");
        let (ok, _) = bounded(command, SYSTEMD_RUN_TIMEOUT)?;
        if ok {
            Ok(())
        } else {
            Err(refused(
                "systemd-run 起 transient unit 失败",
                Verdict::ManagerUnavailable,
            ))
        }
    }

    /// `systemctl show -p …`；按 `Key=Value` 解析成表（输出顺序不按请求顺序，不用 `--value`）。
    ///
    /// # Errors
    /// manager 不可用返回 `ManagerUnavailable`。
    pub fn show(
        &self,
        unit: &str,
        properties: &[&str],
    ) -> Result<BTreeMap<String, String>, Verdict> {
        let selection = format!("{PROPERTY}{}", properties.join(","));
        let output = self.systemctl(&["show", &selection, "--", unit])?;
        Ok(facts::parse_show(&String::from_utf8_lossy(&output)))
    }

    /// 向绑定 unit 的整个控制组发信号（`--kill-whom=all`）；unit 已无进程时是空操作。
    pub fn kill(&self, unit: &str, signal: &str) {
        let _ = self.systemctl(&[
            "kill",
            "--kill-whom=all",
            &format!("--signal={signal}"),
            "--",
            unit,
        ]);
    }

    /// 宽限到期后强停：独立线程等 `grace` 秒再 SIGKILL 整组；unit 早已退出时只是空操作。
    pub fn schedule_kill(&self, unit: &str, grace: u32) {
        let manager = self.clone();
        let unit = unit.to_owned();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(u64::from(grace)));
            manager.kill(&unit, "SIGKILL");
        });
    }

    /// 卸载：`stop` 让 active(exited) 的 transient unit 回收，`reset-failed` 让 failed 的回收。
    pub fn unload(&self, unit: &str) {
        let _ = self.systemctl(&["stop", "--", unit]);
        let _ = self.systemctl(&["reset-failed", "--", unit]);
    }

    fn systemctl(&self, args: &[&str]) -> Result<Vec<u8>, Verdict> {
        let mut command = Command::new(config::SYSTEMCTL);
        self.environment(&mut command);
        if self.user {
            command.arg("--user");
        }
        command.args(args);
        let (ok, output) = bounded(command, SYSTEMCTL_TIMEOUT)?;
        if ok {
            Ok(output)
        } else {
            Err(Verdict::ManagerUnavailable)
        }
    }
}

/// transient unit 的全部属性（设计文档 §4 的固定形状）；用户 manager 只少四项系统专属属性。
fn properties(
    config: &LauncherConfig,
    dir: &SessionDir,
    frame: &LaunchFrame,
    user: bool,
) -> Vec<String> {
    let inaccessible = config
        .inaccessible
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let budget = &frame.budget;
    let mut properties = vec![
        "RemainAfterExit=yes".to_owned(),
        "KillMode=control-group".to_owned(),
        "PrivateMounts=yes".to_owned(),
        "ProtectSystem=strict".to_owned(),
        "ProtectControlGroups=yes".to_owned(),
        "PrivateTmp=yes".to_owned(),
        "NoNewPrivileges=yes".to_owned(),
        "RestrictAddressFamilies=AF_INET AF_INET6".to_owned(),
        format!(
            "ReadWritePaths={} {}",
            config.mirror_root.display(),
            config.payload_config_root.display()
        ),
        format!("InaccessiblePaths={inaccessible}"),
        format!("MemoryMax={}", budget.memory_bytes),
        "MemorySwapMax=0".to_owned(),
        format!("TasksMax={}", budget.pids_max),
        format!("CPUQuota={}%", budget.cpu_max_percent),
        format!("StandardInput=file:{}", dir.release_fifo().display()),
        format!("StandardOutput=truncate:{}", dir.stdout().display()),
        format!("StandardError=truncate:{}", dir.stderr().display()),
    ];
    if user {
        // 垫片二进制在用户 HOME 之下，只能保留只读 HOME。
        properties.push("ProtectHome=read-only".to_owned());
    } else {
        properties.extend(SYSTEM_ONLY.map(str::to_owned));
    }
    properties
}

/// 跑一个已封闭 argv 的子进程：stdin 空、stderr 丢弃、stdout 有界读取；超时即杀。
fn bounded(mut command: Command, timeout: Duration) -> Result<(bool, Vec<u8>), Verdict> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|_| Verdict::ManagerUnavailable)?;
    let stdout = child.stdout.take().ok_or(Verdict::ManagerUnavailable)?;
    // 独立线程有界读完 stdout，子进程写满管道时不会与我们互等。
    let reader = thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stdout.take(OUTPUT_CAP as u64 + 1).read_to_end(&mut buffer);
        buffer
    });
    let deadline = Instant::now() + timeout;
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Verdict::ManagerUnavailable);
            }
        }
    };
    let buffer = reader.join().map_err(|_| Verdict::ManagerUnavailable)?;
    if buffer.len() > OUTPUT_CAP {
        return Err(Verdict::ManagerUnavailable);
    }
    Ok((ok, buffer))
}
