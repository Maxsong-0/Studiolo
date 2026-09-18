//! 已批准能力的只读投影；这里的一切都是「主人批准的期望」，不是「本机已核验的实际」。

use super::error::{Result, TransportError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    num::NonZeroU64,
    path::{Path, PathBuf},
};
use studiolo_core::VaultUuid;

/// 第一片固定的精确发布版本。
pub const PINNED_RUNTIME_VERSION: &str = "0.0.14";

/// 请求绑定事实；它标明「这次请求属于哪条 link」，不构成认证。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportBindingFacts {
    /// canonical Vault 逻辑身份。
    pub vault_uuid: VaultUuid,
    /// 固定同步链身份。
    pub link_id: [u8; 16],
    /// 固定远端 Vault 身份。
    pub remote_vault: [u8; 16],
    /// 非零范围代次；零一律拒绝。
    pub scope_epoch: NonZeroU64,
    /// 完整私有配置实际字节的摘要。
    pub scope_hash: [u8; 32],
    /// 批准的传输审批文档摘要。
    pub approved_config_digest: [u8; 32],
}

/// 两个根的规范路径与观察时刻的真实对象身份。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootPins {
    /// 传输私有根。
    pub private_root: PathBuf,
    /// 传输私有根的 `(dev, ino)`。
    pub private_identity: (u64, u64),
    /// 专用镜像根。
    pub mirror_root: PathBuf,
    /// 镜像根的 `(dev, ino)`。
    pub mirror_identity: (u64, u64),
}

/// 固定运行时事实；不保存任何凭证正文。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePins {
    /// 精确发布版本。
    pub version: String,
    /// 固定 Node 可执行文件绝对路径。
    pub node_path: PathBuf,
    /// 固定 `cli.js` 绝对路径。
    pub cli_path: PathBuf,
    /// 完整只读安装闭包摘要。
    pub closure_digest: [u8; 32],
}

/// 受控后端描述；批准值不代表这些特权在本机已经存在。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendDescriptor {
    /// 精确 systemd unit 名。
    pub service_unit: String,
    /// 专用 payload UID。
    pub payload_uid: u32,
    /// 专用 payload GID。
    pub payload_gid: u32,
    /// 批准的 payload 子组前缀；实测子组必须落在它之内。
    pub payload_cgroup_prefix: String,
    /// 后端配置摘要。
    pub digest: [u8; 32],
}

/// 运行预算；缺项或实际配置不支持则不启动。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBudget {
    /// wall deadline 秒数。
    pub deadline_seconds: u32,
    /// TERM grace 秒数。
    pub stop_grace_seconds: u32,
    /// stdout 字节限额。
    pub stdout_bytes: u32,
    /// stderr 字节限额。
    pub stderr_bytes: u32,
    /// 配置 JSON 字节上限。
    pub config_json_bytes: u32,
    /// 私有状态目录 quota。
    pub private_state_bytes: u64,
    /// `sync.log` 磁盘预算。
    pub log_bytes: u64,
    /// `memory.max` 期望值。
    pub memory_bytes: u64,
    /// `pids.max` 期望值。
    pub pids_max: u32,
    /// `cpu.max` 期望百分比。
    pub cpu_max_percent: u32,
}

impl RunBudget {
    /// 预算缺项（任一为零）即不完整；不完整不得启动。
    #[must_use]
    pub fn complete(&self) -> bool {
        self.deadline_seconds > 0
            && self.stop_grace_seconds > 0
            && self.stdout_bytes > 0
            && self.stderr_bytes > 0
            && self.config_json_bytes > 0
            && self.private_state_bytes > 0
            && self.log_bytes > 0
            && self.memory_bytes > 0
            && self.pids_max > 0
            && self.cpu_max_percent > 0
    }
}

/// 已批准传输能力的唯一来源。
///
/// 实现者必须真实复验私有配置快照与根身份；本 crate 不自己打开配置文件。
pub trait ApprovedTransport {
    /// 复验私有配置与根身份；任何漂移都必须报错。
    ///
    /// # Errors
    /// 配置正文、对象身份或根身份变化时返回 `ConfigurationChanged`/`RootsChanged`。
    fn revalidate(&self) -> Result<()>;

    /// 请求绑定事实。
    ///
    /// # Errors
    /// 范围代次为零时返回 `BindingMismatch`。
    fn binding(&self) -> Result<TransportBindingFacts>;

    /// 两个根的规范路径与对象身份。
    fn roots(&self) -> RootPins;

    /// 固定运行时事实。
    fn runtime(&self) -> RuntimePins;

    /// 受控后端描述。
    fn backend(&self) -> BackendDescriptor;

    /// 运行预算。
    fn budget(&self) -> RunBudget;
}

/// 已固定的运行时；没有公开构造器，也不能被反序列化。
///
/// ```compile_fail
/// let runtime: studiolo_sync_transport::owned_run::PinnedRuntime =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedRuntime {
    pins: RuntimePins,
    digest: [u8; 32],
}

impl PinnedRuntime {
    /// 只从已批准能力取已核验的运行时事实；本类型自己不打开任何文件。
    ///
    /// # Errors
    /// 版本不是精确发布版本、路径非绝对规范或两条路径相同时返回 `RuntimeChanged`。
    pub fn from_approval(approval: &dyn ApprovedTransport) -> Result<Self> {
        approval.revalidate()?;
        let pins = approval.runtime();
        if pins.version != PINNED_RUNTIME_VERSION
            || !absolute_normal(&pins.node_path)
            || !absolute_normal(&pins.cli_path)
            || pins.node_path == pins.cli_path
        {
            return Err(TransportError::RuntimeChanged);
        }
        let digest = runtime_digest(&pins);
        Ok(Self { pins, digest })
    }

    /// 运行时摘要；进入持久记录并在每次使用前后复验。
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }

    /// 只读运行时事实。
    #[must_use]
    pub fn pins(&self) -> &RuntimePins {
        &self.pins
    }
}

/// 对固定运行时事实的域分隔摘要。
#[must_use]
pub fn runtime_digest(pins: &RuntimePins) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"studiolo.sync-transport.runtime.v1\0");
    for field in [
        pins.version.as_bytes(),
        pins.node_path.as_os_str().as_encoded_bytes(),
        pins.cli_path.as_os_str().as_encoded_bytes(),
    ] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field);
    }
    hash.update(pins.closure_digest);
    hash.finalize().into()
}

/// 绝对、无 `.`/`..`、无控制字符的路径。
#[must_use]
pub fn absolute_normal(path: &Path) -> bool {
    path.to_str().is_some_and(|value| {
        value.starts_with('/')
            && value.len() > 1
            && !value.chars().any(char::is_control)
            && value[1..]
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
    })
}
