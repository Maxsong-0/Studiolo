//! launcher 自己的固定配置（`/etc/studiolo/sync-launcher.json`，0600 root）：与 owner 的审批
//! 文档互相独立、互相核对。任何可执行路径、镜像路径、身份都只从这里来，不从 IPC 来。

use serde::Deserialize;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use studiolo_sync_transport::owned_run::RuntimePins;

/// 配置文件的固定路径。
pub const CONFIG_PATH: &str = "/etc/studiolo/sync-launcher.json";
/// 本程序的固定安装路径；transient unit 的 `ExecStart` 只指向它，不从 IPC 或环境取。
pub const LAUNCHER_BINARY: &str = "/usr/local/sbin/studiolo-sync-launcher";
/// 私有 socket 目录；0750 root:<owner 组>。
pub const SOCKET_DIR: &str = "/run/studiolo-sync-launcher";
/// 每个 session 的 FIFO 目录。
pub const SESSIONS_DIR: &str = "/run/studiolo-sync-launcher/sessions";
/// `systemd-run` 的固定绝对路径；不做 PATH 搜索。
pub const SYSTEMD_RUN: &str = "/usr/bin/systemd-run";
/// `systemctl` 的固定绝对路径。
pub const SYSTEMCTL: &str = "/usr/bin/systemctl";
/// 配置字节上限。
pub const CONFIG_MAX_BYTES: usize = 16 * 1024;

/// 固定配置的失败分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigFault {
    /// 文件不存在或不是 0600 root 普通文件。
    Unsafe,
    /// JSON 形状不合法或字段自相矛盾。
    Malformed,
}

/// 已解析的固定配置；没有公开构造器。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherConfig {
    /// 固定为 1。
    pub schema_version: u8,
    /// 允许连接 socket 的 owner UID（`studiolo-human` 进程）。
    pub owner_uid: u32,
    /// owner GID（socket 目录的组）。
    pub owner_gid: u32,
    /// 专用 payload 身份（`studiolo-sync`）。
    pub payload_uid: u32,
    /// 专用 payload 组。
    pub payload_gid: u32,
    /// 固定运行时：node、cli.js、版本、闭包摘要。
    pub runtime: RuntimePins,
    /// payload 视图内的真实镜像根。
    pub mirror_root: PathBuf,
    /// headless 配置根（`XDG_CONFIG_HOME`）。
    pub payload_config_root: PathBuf,
    /// 固定 loop 镜像文件；`disk_quota_enforced` 以它为据。
    pub loop_image: PathBuf,
    /// loop 镜像挂载点（`/var/lib/studiolo-sync`）。
    pub loop_mount: PathBuf,
    /// 必须对 payload 不可达的路径（canonical、writer-data、Bridge DB、外 stage、`/home`）。
    pub inaccessible: Vec<PathBuf>,
    /// 批准的 payload 子组前缀。
    pub payload_cgroup_prefix: PathBuf,
}

impl LauncherConfig {
    /// 解析并校验固定配置字节。
    ///
    /// # Errors
    /// 形状或数值不合法返回 `Malformed`。
    pub fn parse(bytes: &[u8]) -> Result<Self, ConfigFault> {
        if bytes.len() > CONFIG_MAX_BYTES {
            return Err(ConfigFault::Malformed);
        }
        let config: Self = serde_json::from_slice(bytes).map_err(|_| ConfigFault::Malformed)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigFault> {
        use studiolo_sync_transport::owned_run::absolute_normal;
        let identities_ok = self.schema_version == 1
            && self.owner_uid != 0
            && self.payload_uid != 0
            && self.owner_uid != self.payload_uid
            && self.owner_gid != self.payload_gid;
        let paths_ok = [
            &self.mirror_root,
            &self.payload_config_root,
            &self.loop_image,
            &self.loop_mount,
            &self.payload_cgroup_prefix,
            &self.runtime.node_path,
            &self.runtime.cli_path,
        ]
        .into_iter()
        .chain(self.inaccessible.iter())
        .all(|path| absolute_normal(path));
        let layout_ok = self.mirror_root.starts_with(&self.loop_mount)
            && self.payload_config_root.starts_with(&self.loop_mount)
            && !self.loop_image.starts_with(&self.loop_mount)
            && self.inaccessible.len() <= 64
            && self.runtime.version == studiolo_sync_transport::owned_run::PINNED_RUNTIME_VERSION;
        if identities_ok && paths_ok && layout_ok {
            Ok(())
        } else {
            Err(ConfigFault::Malformed)
        }
    }

    /// 读取固定路径的配置；须 0600、root 所有、普通单链接文件。
    ///
    /// # Errors
    /// 文件不满足约束返回 `Unsafe`；内容不合法返回 `Malformed`。
    pub fn load() -> Result<Self, ConfigFault> {
        // 只看链接本身（不跟随）：符号链接、非普通文件、非 root、非 0600、多链接或超预算一律不安全。
        let metadata = std::fs::symlink_metadata(CONFIG_PATH).map_err(|_| ConfigFault::Unsafe)?;
        let safe = metadata.is_file()
            && metadata.uid() == 0
            && metadata.mode() & 0o777 == 0o600
            && metadata.nlink() == 1
            && metadata.len() <= CONFIG_MAX_BYTES as u64;
        if !safe {
            return Err(ConfigFault::Unsafe);
        }
        let bytes = std::fs::read(CONFIG_PATH).map_err(|_| ConfigFault::Unsafe)?;
        Self::parse(&bytes)
    }
}
