//! 有界审批文档解析；默认关闭，未显式启用不产生任何能力。
//!
//! 本模块只做纯数据校验：不打开文件、不读环境变量、不解析凭证，
//! 也不把批准值当作平台已具备该特权的证据。

use super::types::{
    ManualQuota, TransportBackend, TransportBudget, TransportRuntime, TransportSyncMode,
};
use crate::{ConfigError, absolute_normal};
use serde::Deserialize;
use std::path::PathBuf;

mod digest;
mod validate;
pub use validate::excluded_folder as excluded_folder_acceptable;
use validate::{backend, budget, device_name, excluded_folders, manual_quota, runtime};

/// 审批文档字节上限；单文档远小于私有配置预算。
const APPROVAL_MAX_BYTES: usize = 8 * 1024;
/// 排除文件夹条数上限；headless `--excluded-folders` 是逐项列出的。
pub(super) const EXCLUDED_FOLDERS_MAX: usize = 64;
/// 第一片只固定这一个精确发布版本。
pub(super) const PINNED_VERSION: &str = "0.0.14";
/// InspectConfig 的 JSON 上限，与执行契约的 64 KiB 一致。
pub(super) const CONFIG_JSON_MAX: u32 = 64 * 1024;
/// 受控 payload 子组只能落在 cgroup v2 挂载点之内。
pub(super) const CGROUP_ROOT: &str = "/sys/fs/cgroup/";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u8,
    enabled: bool,
    private_root: PathBuf,
    mirror_root: PathBuf,
    #[serde(rename = "owner_view_root")]
    mirror_view_root: PathBuf,
    plane_root: PathBuf,
    device_name: String,
    headless_mode: String,
    excluded_folders: Vec<String>,
    // 键必须显式出现：`null` 是「配额未知」的明确申报，漏写整个键则拒绝。
    #[serde(deserialize_with = "explicit_quota")]
    manual_quota: Option<ManualQuotaDocument>,
    runtime: RuntimeDocument,
    backend: BackendDocument,
    budget: BudgetDocument,
}

/// 绕开 serde 对 `Option` 字段「缺键即 `None`」的默认处理：键缺失按缺字段拒绝。
fn explicit_quota<'de, D: serde::Deserializer<'de>>(
    input: D,
) -> Result<Option<ManualQuotaDocument>, D::Error> {
    Deserialize::deserialize(input)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManualQuotaDocument {
    account_limit_bytes: u64,
    used_bytes_manual: u64,
    recorded_at_unix: i64,
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
    mirror_view_root: PathBuf,
    plane_root: PathBuf,
    device_name: String,
    headless_mode: TransportSyncMode,
    excluded_folders: Vec<String>,
    manual_quota: Option<ManualQuota>,
    runtime: TransportRuntime,
    backend: TransportBackend,
    budget: TransportBudget,
    digest: [u8; 32],
}

impl SyncTransportApproval {
    /// 从 0600 私有文件读取并解析审批文档；父目录须 0700、本人所有（与私有配置同一读法）。
    ///
    /// # Errors
    /// 文件不满足私有约束或内容不合法返回 `ConfigError`。
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        let snapshot = crate::config_file::Snapshot::read(path)?;
        Self::parse(snapshot.bytes())
    }

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
        let roots = [
            &document.private_root,
            &document.mirror_root,
            &document.mirror_view_root,
            &document.plane_root,
        ];
        // 四个根两两不同名；真实对象不重叠由 `ApprovedTransport::approve` 用 (dev,ino) 再核。
        if roots.iter().any(|root| !absolute_normal(root))
            || (1..roots.len()).any(|i| roots[..i].contains(&roots[i]))
        {
            return Err(ConfigError);
        }
        let headless_mode = match document.headless_mode.as_str() {
            "mirror-remote" => TransportSyncMode::MirrorRemote,
            "bidirectional" => TransportSyncMode::Bidirectional,
            _ => return Err(ConfigError),
        };
        let excluded_folders = excluded_folders(&document.excluded_folders)?;
        if !device_name(&document.device_name) {
            return Err(ConfigError);
        }
        let manual_quota = document
            .manual_quota
            .as_ref()
            .map(manual_quota)
            .transpose()?;
        let digest = digest::transport_digest(
            [
                &document.private_root,
                &document.mirror_root,
                &document.mirror_view_root,
                &document.plane_root,
            ],
            &runtime,
            &backend,
            budget,
        );
        Ok(Self {
            private_root: document.private_root,
            mirror_root: document.mirror_root,
            mirror_view_root: document.mirror_view_root,
            plane_root: document.plane_root,
            device_name: document.device_name,
            headless_mode,
            excluded_folders,
            manual_quota,
            runtime,
            backend,
            budget,
            digest,
        })
    }

    pub(super) fn private_root(&self) -> &std::path::Path {
        &self.private_root
    }

    pub(super) fn mirror_root(&self) -> &std::path::Path {
        &self.mirror_root
    }

    pub(super) fn mirror_view_root(&self) -> &std::path::Path {
        &self.mirror_view_root
    }

    pub(super) fn plane_root(&self) -> &std::path::Path {
        &self.plane_root
    }

    /// 镜像节点的设备名；核对 headless `deviceName` 用。
    #[must_use]
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// 审批的镜像同步模式。
    #[must_use]
    pub fn headless_mode(&self) -> TransportSyncMode {
        self.headless_mode
    }

    /// 审批的排除文件夹；这是唯一母本（ADR-025 D10），headless 配置必须逐项相等。
    #[must_use]
    pub fn excluded_folders(&self) -> &[String] {
        &self.excluded_folders
    }

    /// 人工录入的账号配额；`None` 即该健康维度未知。
    #[must_use]
    pub fn manual_quota(&self) -> Option<ManualQuota> {
        self.manual_quota
    }

    pub(super) fn parts(&self) -> (&TransportRuntime, &TransportBackend, TransportBudget) {
        (&self.runtime, &self.backend, self.budget)
    }

    /// 批准配置摘要：只覆盖传输安全相关字段（四个根、运行时、后端、预算），进入传输私有存储的
    /// 持久绑定；平面字段改动不影响它（见 `document/digest.rs`）。
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
}
