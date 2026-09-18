//! 有界审批文档解析；默认关闭，未显式启用不产生任何能力。
//!
//! 本模块只做纯数据校验：不打开文件、不读环境变量、不解析凭证，
//! 也不把批准值当作平台已具备该特权的证据。

use super::types::{TransportBackend, TransportBudget, TransportRuntime};
use crate::{ConfigError, absolute_normal};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// 审批文档字节上限；单文档远小于私有配置预算。
const APPROVAL_MAX_BYTES: usize = 8 * 1024;
/// 第一片只固定这一个精确发布版本。
const PINNED_VERSION: &str = "0.0.14";
/// InspectConfig 的 JSON 上限，与执行契约的 64 KiB 一致。
const CONFIG_JSON_MAX: u32 = 64 * 1024;
/// 受控 payload 子组只能落在 cgroup v2 挂载点之内。
const CGROUP_ROOT: &str = "/sys/fs/cgroup/";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u8,
    enabled: bool,
    private_root: PathBuf,
    mirror_root: PathBuf,
    runtime: RuntimeDocument,
    backend: BackendDocument,
    budget: BudgetDocument,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeDocument {
    node_path: PathBuf,
    cli_path: PathBuf,
    version: String,
    closure_digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackendDocument {
    service_unit: String,
    launcher_path: PathBuf,
    payload_cgroup_prefix: PathBuf,
    payload_uid: u32,
    payload_gid: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetDocument {
    deadline_seconds: u32,
    stop_grace_seconds: u32,
    stdout_bytes: u32,
    stderr_bytes: u32,
    config_json_bytes: u32,
    private_state_bytes: u64,
    log_bytes: u64,
    memory_bytes: u64,
    swap_bytes: u64,
    pids_max: u32,
    cpu_max_percent: u32,
}

/// 已解析并逐字段校验的传输审批；没有公开构造器或字段。
///
/// ```compile_fail
/// let approval: studiolo_host_config::sync_transport::SyncTransportApproval =
///     serde_json::from_str("{}").unwrap();
/// ```
pub struct SyncTransportApproval {
    private_root: PathBuf,
    mirror_root: PathBuf,
    runtime: TransportRuntime,
    backend: TransportBackend,
    budget: TransportBudget,
    digest: [u8; 32],
}

impl SyncTransportApproval {
    /// 解析主人显式提供的审批文档字节。
    ///
    /// 只接受 schema 1、显式 `enabled = true` 的命名字段对象；未知字段、位置数组、
    /// 尾随对象、超预算输入一律拒绝。成功只代表这些值形式合法且自洽，
    /// 不代表对应的 systemd/cgroup/身份特权在本机已经存在。
    pub fn parse(bytes: &[u8]) -> Result<Self, ConfigError> {
        if bytes.len() > APPROVAL_MAX_BYTES {
            return Err(ConfigError);
        }
        let first = bytes
            .iter()
            .copied()
            .find(|byte| !matches!(*byte, b' ' | b'\n' | b'\r' | b'\t'));
        if first != Some(b'{') {
            return Err(ConfigError);
        }
        let document: Document = serde_json::from_slice(bytes).map_err(|_| ConfigError)?;
        if document.schema_version != 1 || !document.enabled {
            return Err(ConfigError);
        }
        let runtime = runtime(&document.runtime)?;
        let backend = backend(&document.backend)?;
        let budget = budget(&document.budget)?;
        if !absolute_normal(&document.private_root) || !absolute_normal(&document.mirror_root) {
            return Err(ConfigError);
        }
        let mut hash = Sha256::new();
        hash.update(b"studiolo.sync-transport.approval.v1\0");
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
        Ok(Self {
            private_root: document.private_root,
            mirror_root: document.mirror_root,
            runtime,
            backend,
            budget,
            digest: hash.finalize().into(),
        })
    }

    pub(super) fn private_root(&self) -> &std::path::Path {
        &self.private_root
    }

    pub(super) fn mirror_root(&self) -> &std::path::Path {
        &self.mirror_root
    }

    pub(super) fn parts(&self) -> (&TransportRuntime, &TransportBackend, TransportBudget) {
        (&self.runtime, &self.backend, self.budget)
    }

    /// 域分隔后对审批文档实际字节的 SHA-256；批准配置摘要进入持久绑定。
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

fn runtime(document: &RuntimeDocument) -> Result<TransportRuntime, ConfigError> {
    if document.version != PINNED_VERSION
        || !absolute_normal(&document.node_path)
        || !absolute_normal(&document.cli_path)
        || document.node_path == document.cli_path
    {
        return Err(ConfigError);
    }
    Ok(TransportRuntime::new(
        document.node_path.clone(),
        document.cli_path.clone(),
        document.version.clone(),
        digest32(&document.closure_digest)?,
    ))
}

fn backend(document: &BackendDocument) -> Result<TransportBackend, ConfigError> {
    let unit = document.service_unit.as_str();
    let unit_ok = unit.len() <= 128
        && unit.ends_with(".service")
        && unit.len() > ".service".len()
        && unit
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'.' | b'_' | b'-'));
    let cgroup_ok = absolute_normal(&document.payload_cgroup_prefix)
        && document
            .payload_cgroup_prefix
            .to_str()
            .is_some_and(|value| value.starts_with(CGROUP_ROOT) && value.len() > CGROUP_ROOT.len());
    // payload 身份必须与 owner 分开；root 或 owner 自身身份都不构成隔离。
    let owner = rustix::process::geteuid().as_raw();
    let identity_ok = document.payload_uid != 0
        && document.payload_gid != 0
        && document.payload_uid != owner
        && document.payload_gid != rustix::process::getegid().as_raw();
    if !unit_ok || !cgroup_ok || !identity_ok || !absolute_normal(&document.launcher_path) {
        return Err(ConfigError);
    }
    Ok(TransportBackend::new(
        document.service_unit.clone(),
        document.launcher_path.clone(),
        document.payload_cgroup_prefix.clone(),
        document.payload_uid,
        document.payload_gid,
    ))
}

fn budget(document: &BudgetDocument) -> Result<TransportBudget, ConfigError> {
    let bounded = (1..=86_400).contains(&document.deadline_seconds)
        && (1..=300).contains(&document.stop_grace_seconds)
        && (1..=16 * 1024 * 1024).contains(&document.stdout_bytes)
        && (1..=16 * 1024 * 1024).contains(&document.stderr_bytes)
        && (1..=CONFIG_JSON_MAX).contains(&document.config_json_bytes)
        && (1..=4096).contains(&document.pids_max)
        && (1..=6400).contains(&document.cpu_max_percent)
        && document.private_state_bytes > 0
        && document.log_bytes > 0
        && document.memory_bytes > 0;
    // 不给 payload 任何 swap 预算：内存上限必须可核验且不被换页绕开。
    if !bounded || document.swap_bytes != 0 {
        return Err(ConfigError);
    }
    Ok(TransportBudget {
        deadline_seconds: document.deadline_seconds,
        stop_grace_seconds: document.stop_grace_seconds,
        stdout_bytes: document.stdout_bytes,
        stderr_bytes: document.stderr_bytes,
        config_json_bytes: document.config_json_bytes,
        private_state_bytes: document.private_state_bytes,
        log_bytes: document.log_bytes,
        memory_bytes: document.memory_bytes,
        pids_max: document.pids_max,
        cpu_max_percent: document.cpu_max_percent,
    })
}

fn digest32(value: &str) -> Result<[u8; 32], ConfigError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ConfigError);
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(ConfigError);
    }
    let mut output = [0u8; 32];
    for (index, slot) in output.iter_mut().enumerate() {
        let pair = value.get(index * 2..index * 2 + 2).ok_or(ConfigError)?;
        *slot = u8::from_str_radix(pair, 16).map_err(|_| ConfigError)?;
    }
    Ok(output)
}
