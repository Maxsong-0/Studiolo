//! 审批文档各段的纯校验；只看数值形状，不证明本机特权存在。

use super::{
    BackendDocument, BudgetDocument, CGROUP_ROOT, CONFIG_JSON_MAX, EXCLUDED_FOLDERS_MAX,
    ManualQuotaDocument, PINNED_VERSION, RuntimeDocument,
};
use crate::sync_transport::types::{
    ManualQuota, TransportBackend, TransportBudget, TransportRuntime,
};
use crate::{ConfigError, absolute_normal};

pub(super) fn runtime(document: &RuntimeDocument) -> Result<TransportRuntime, ConfigError> {
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

pub(super) fn backend(document: &BackendDocument) -> Result<TransportBackend, ConfigError> {
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

pub(super) fn budget(document: &BudgetDocument) -> Result<TransportBudget, ConfigError> {
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

/// 单个排除文件夹名是否合法：Vault 相对路径，不带前后斜杠、不含 `.`/`..` 段、
/// 无反斜杠与控制字符、非空白、不超过 1024 字节。
///
/// 这是唯一母本：审批文档解析与 `studiolo-maintain sync-purge-residue --folder` 都用它，
/// 不各抄一份规则。
#[must_use]
pub fn excluded_folder(value: &str) -> bool {
    text_field(value, 1024)
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains('\\')
        && !value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

/// 排除文件夹清单：逐项合法、不重复、条数不超过上限；返回值已排序。
pub(super) fn excluded_folders(values: &[String]) -> Result<Vec<String>, ConfigError> {
    if values.len() > EXCLUDED_FOLDERS_MAX {
        return Err(ConfigError);
    }
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        if !excluded_folder(value) || !seen.insert(value.as_str()) {
            return Err(ConfigError);
        }
    }
    let mut sorted = values.to_vec();
    sorted.sort();
    Ok(sorted)
}

pub(super) fn manual_quota(document: &ManualQuotaDocument) -> Result<ManualQuota, ConfigError> {
    if document.account_limit_bytes == 0
        || document.used_bytes_manual > document.account_limit_bytes
        || document.recorded_at_unix <= 0
    {
        return Err(ConfigError);
    }
    Ok(ManualQuota {
        account_limit_bytes: document.account_limit_bytes,
        used_bytes_manual: document.used_bytes_manual,
        recorded_at_unix: document.recorded_at_unix,
    })
}

/// 设备名最多的字符数；与 local-server `sync_plane::naming` 生成冲突副本时的设备名上限一致。
pub(super) const DEVICE_NAME_MAX_CHARS: usize = 64;

/// 设备名：非空行、无控制字符、至多 [`DEVICE_NAME_MAX_CHARS`] 个字符，且不含 `(`、`)`、`/`。
/// 它既要逐字等于 headless 的 `deviceName`，又要原样进冲突副本名
/// `<名> (Conflicted copy <设备名> <戳>).<ext>`——括号与斜杠会让副本名解析不回来。
/// 在审批层拒绝，而不是让 Bidirectional 的冲突保全每轮在命名处失败。
pub(super) fn device_name(value: &str) -> bool {
    text_field(value, 256)
        && value.chars().count() <= DEVICE_NAME_MAX_CHARS
        && !value.contains(['(', ')', '/'])
}

pub(super) fn text_field(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

pub(super) fn digest32(value: &str) -> Result<[u8; 32], ConfigError> {
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
