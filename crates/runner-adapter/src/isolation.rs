//! Linux user namespace 与 cgroup 隔离后端；可用性必须由实时探针证明。
use crate::{AdapterAvailability, AdapterProbe, RunnerError};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Linux 后端不可变的部署输入；各路径归管理员所有。
#[derive(Clone, Debug)]
pub struct LinuxIsolation {
    /// 最小的只读运行时根目录，内含钉死版本的 CLI、Python 与探针脚本。
    pub runtime_root: PathBuf,
    /// 由管理员安装的 Bubblewrap 可执行文件。
    pub bubblewrap: PathBuf,
    /// 用户级 systemd 客户端；服务账号必须有处于活动状态的用户管理器。
    pub systemd_run: PathBuf,
    /// 唯一可向外通达的传输：本机 Gateway 的 Unix socket。
    pub gateway_socket: PathBuf,
}
impl LinuxIsolation {
    pub(crate) fn command(&self, unit: &str, work: &Path, inputs: &Path, seconds: u64) -> Command {
        let mut command = Command::new(&self.systemd_run);
        command.env_clear().env("PATH", "/usr/bin:/bin");
        // 除定位服务账号用户管理器所需的变量外，不继承任何环境变量。
        if let Some(value) = std::env::var_os("XDG_RUNTIME_DIR") {
            command.env("XDG_RUNTIME_DIR", value);
        }
        if let Some(value) = std::env::var_os("DBUS_SESSION_BUS_ADDRESS") {
            command.env("DBUS_SESSION_BUS_ADDRESS", value);
        }
        command
            .args(["--user", "--scope", "--quiet", "--collect", "--unit"])
            .arg(unit)
            .arg("--property=MemoryMax=1073741824")
            .arg("--property=MemorySwapMax=0")
            .arg("--property=TasksMax=64")
            .arg("--property=CPUQuota=200%")
            .arg(format!(
                "--property=RuntimeMaxSec={}",
                seconds.saturating_add(180)
            ))
            .arg(&self.bubblewrap)
            .args([
                "--unshare-user",
                "--unshare-ipc",
                "--unshare-pid",
                "--unshare-net",
                "--unshare-uts",
                "--die-with-parent",
                "--new-session",
                "--uid",
                "65534",
                "--gid",
                "65534",
                "--cap-drop",
                "ALL",
                "--clearenv",
                "--ro-bind",
            ])
            .arg(&self.runtime_root)
            .arg("/")
            .args(["--ro-bind", "/sys/fs/cgroup", "/sys/fs/cgroup"])
            .args(["--bind"])
            .arg(work)
            .arg("/work")
            .args(["--ro-bind"])
            .arg(inputs)
            .arg("/inputs")
            // 不可变的 rootfs 无需预置 socket 占位文件：先在私有、有容量上限的
            // /run 中创建挂载点，再把 socket 绑定进去。
            .args(["--size", "1048576", "--tmpfs", "/run"])
            .args(["--ro-bind"])
            .arg(&self.gateway_socket)
            .arg("/run/studiolo-gateway.sock")
            .args([
                "--size",
                "67108864",
                "--tmpfs",
                "/home",
                "--dir",
                "/home/runner",
                "--size",
                "67108864",
                "--tmpfs",
                "/tmp",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--chdir",
                "/work",
                "--setenv",
                "HOME",
                "/home/runner",
                "--setenv",
                "PATH",
                "/usr/local/bin:/usr/bin:/bin",
                "--setenv",
                "LANG",
                "C.UTF-8",
            ]);
        command
    }
    /// 在两个适配器共用的同一沙箱中，运行随部署固定的反向探针（验证越权操作确实被拦住）。
    /// 调用方可控的布尔值或旧回执，都无法让它返回 Ready。
    pub fn probe(&self) -> Result<AdapterAvailability, RunnerError> {
        self.probe_at(&std::env::temp_dir())
    }
    /// 在 attempt 实际使用的同一个有界文件系统上运行实时探针。
    pub fn probe_at(&self, scratch_root: &Path) -> Result<AdapterAvailability, RunnerError> {
        self.probe_profile(scratch_root, None)
            .map(|r| r.availability)
    }
    pub(crate) fn probe_cli_at(
        &self,
        scratch_root: &Path,
        provider: crate::CliProvider,
    ) -> Result<AdapterProbe, RunnerError> {
        self.probe_profile(scratch_root, Some(provider))
    }
    fn probe_profile(
        &self,
        scratch_root: &Path,
        provider: Option<crate::CliProvider>,
    ) -> Result<AdapterProbe, RunnerError> {
        let unavailable = || AdapterProbe {
            availability: AdapterAvailability::Unavailable {
                reason: "live RG-RUNNER probe or deployment prerequisites unavailable".into(),
            },
            binary_version: None,
            protocol_capabilities: Vec::new(),
            missing_prerequisites: vec![
                "pinned rootfs, CLI protocol, namespace/cgroup/scratch or relay boundary".into(),
            ],
        };
        if !cfg!(target_os = "linux") {
            return Ok(unavailable());
        }
        for path in [&self.runtime_root, &self.bubblewrap, &self.systemd_run] {
            let Ok(meta) = path.symlink_metadata() else {
                return Ok(unavailable());
            };
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if meta.uid() != 0 || meta.mode() & 0o022 != 0 || meta.file_type().is_symlink() {
                    return Ok(unavailable());
                }
            }
        }
        let probe = self.runtime_root.join("usr/lib/studiolo/rg_probe.py");
        let expected = include_bytes!("../../../deploy/runner/rg_probe.py");
        if std::fs::read(probe).ok().as_deref() != Some(expected.as_slice()) {
            return Ok(unavailable());
        }
        let temp = tempfile::Builder::new()
            .prefix("rg-probe-")
            .tempdir_in(scratch_root)
            .map_err(|e| RunnerError::io("probe scratch", e))?;
        let mut command = self.command(
            &format!("studiolo-probe-{}", std::process::id()),
            temp.path(),
            temp.path(),
            20,
        );
        let before = std::fs::read_link("/proc/self/ns/net")
            .map_err(|e| RunnerError::io("network identity", e))?;
        command
            .args(["/usr/bin/python3", "/usr/lib/studiolo/rg_probe.py"])
            .arg(before)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(provider) = provider {
            command.arg(provider.name());
        }
        let Ok(mut child) = command.spawn() else {
            return Ok(unavailable());
        };
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        return Ok(unavailable());
                    }
                    let mut report = AdapterProbe {
                        availability: AdapterAvailability::Ready,
                        binary_version: None,
                        protocol_capabilities: Vec::new(),
                        missing_prerequisites: Vec::new(),
                    };
                    if provider.is_some() {
                        use std::io::Read;
                        #[derive(serde::Deserialize)]
                        #[serde(deny_unknown_fields)]
                        struct Wire {
                            binary_version: String,
                            protocol_capabilities: Vec<String>,
                        }
                        let mut bytes = Vec::new();
                        let Some(output) = child.stdout.take() else {
                            return Ok(unavailable());
                        };
                        if output.take(4097).read_to_end(&mut bytes).is_err() || bytes.len() > 4096
                        {
                            return Ok(unavailable());
                        }
                        let Ok(wire) = serde_json::from_slice::<Wire>(&bytes) else {
                            return Ok(unavailable());
                        };
                        if wire.binary_version.is_empty()
                            || wire.binary_version.len() > 256
                            || wire.binary_version.chars().any(char::is_control)
                            || wire.protocol_capabilities.is_empty()
                        {
                            return Ok(unavailable());
                        }
                        report.binary_version = Some(wire.binary_version);
                        report.protocol_capabilities = wire.protocol_capabilities;
                    }
                    return Ok(report);
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(unavailable());
                }
            }
        }
    }
    pub(crate) fn stop_unit(&self, unit: &str, signal: &str) -> Result<(), RunnerError> {
        // unit 名只由 RealRunner 依据不透明的 attempt id 与 fence 构造。
        let mut child = Command::new("systemctl")
            .args([
                "--user",
                "kill",
                "--kill-whom=all",
                "--signal",
                signal,
                unit,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| RunnerError::Unavailable)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(RunnerError::Unavailable)
                    };
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(RunnerError::Unavailable);
                }
            }
        }
    }
}
