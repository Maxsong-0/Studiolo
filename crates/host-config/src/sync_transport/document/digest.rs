//! 批准配置摘要：只覆盖传输安全相关的字段——四个根、固定运行时、受控后端与运行预算。
//!
//! 平面字段（`device_name`、`headless_mode`、`excluded_folders`、`manual_quota`）不进摘要：
//! 前三项每轮都由驱动器经 InspectConfig 按审批值逐项核对，`manual_quota` 只作健康展示，
//! 改动后重启 human 即生效。若把它们也绑进传输私有存储的持久绑定，排除项变更、切到
//! bidirectional、更新人工配额都会让 human 起不来（2026-09-26 T-132 P1 遗留评审）。
//! 字段一律按固定顺序「长度＋字节」编码，整数按大端 u64，不依赖文档里的键序、空白或注释。

use crate::sync_transport::types::{TransportBackend, TransportBudget, TransportRuntime};
use sha2::{Digest, Sha256};
use std::path::Path;

/// 域分隔前缀；v2 起只覆盖传输安全字段（v1 是整份文档字节）。
const DOMAIN: &[u8] = b"studiolo.sync-transport.approval.v2\0";

/// 计算批准配置摘要；`roots` 依次是传输私有根、镜像根、镜像视图根、平面根。
pub(super) fn transport_digest(
    roots: [&Path; 4],
    runtime: &TransportRuntime,
    backend: &TransportBackend,
    budget: TransportBudget,
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(DOMAIN);
    for root in roots {
        field(&mut hash, root.as_os_str().as_encoded_bytes());
    }
    field(&mut hash, runtime.version().as_bytes());
    field(
        &mut hash,
        runtime.node_path().as_os_str().as_encoded_bytes(),
    );
    field(&mut hash, runtime.cli_path().as_os_str().as_encoded_bytes());
    field(&mut hash, &runtime.closure_digest());
    field(&mut hash, backend.service_unit().as_bytes());
    field(
        &mut hash,
        backend.launcher_path().as_os_str().as_encoded_bytes(),
    );
    field(
        &mut hash,
        backend
            .payload_cgroup_prefix()
            .as_os_str()
            .as_encoded_bytes(),
    );
    for number in [
        u64::from(backend.payload_uid()),
        u64::from(backend.payload_gid()),
        u64::from(budget.deadline_seconds),
        u64::from(budget.stop_grace_seconds),
        u64::from(budget.stdout_bytes),
        u64::from(budget.stderr_bytes),
        u64::from(budget.config_json_bytes),
        budget.private_state_bytes,
        budget.log_bytes,
        budget.memory_bytes,
        u64::from(budget.pids_max),
        u64::from(budget.cpu_max_percent),
    ] {
        hash.update(number.to_be_bytes());
    }
    hash.finalize().into()
}

/// 一个变长字段：长度（大端 u64）后接字节。
fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}
