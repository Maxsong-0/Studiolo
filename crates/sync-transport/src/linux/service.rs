//! systemd 侧的纯策略：固定 unit 名、封闭 argv，且 service 状态永远不是静止证明。

use super::ContainmentGap;
use crate::owned_run::{RunOperation, RuntimePins};
use std::path::Path;

/// 精确 unit 名的封闭形状；owner 不接受任意 unit 或属性。
#[must_use]
pub fn unit_name_acceptable(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.ends_with(".service")
        && name.len() > ".service".len()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'.' | b'_' | b'-'))
}

/// 从 manager 读到的 unit 事实；它只用于诊断与身份比对。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceProbe {
    /// unit 名。
    pub unit_name: String,
    /// `ActiveState`。
    pub active_state: String,
    /// `MainPID`。
    pub main_pid: u32,
    /// manager 侧 InvocationID。
    pub invocation_id: [u8; 16],
}

/// service 状态永远不是静止证明。
///
/// `MainPID=0`、`ActiveState=inactive`、读不到 unit、`Child.wait`、PGID 与 pipe EOF
/// 都不能替代「同一受控 payload 子组实测为空」。本函数存在就是为了把这条写进代码里。
#[must_use]
pub fn is_quiescence_evidence(_probe: &ServiceProbe) -> bool {
    false
}

/// 同名 unit 是否确实是同一次调用；InvocationID 变化即视为已被替换。
#[must_use]
pub fn same_invocation(probe: &ServiceProbe, unit_name: &str, invocation_id: [u8; 16]) -> bool {
    probe.unit_name == unit_name && probe.invocation_id == invocation_id
}

/// 封闭 argv：固定绝对路径的 Node 与 cli.js，禁 shell、禁 PATH 搜索、禁环境继承。
///
/// # 错误
/// 路径非绝对规范、镜像根非绝对规范时返回 `ProtocolViolation`。
pub fn build_argv(
    runtime: &RuntimePins,
    operation: RunOperation,
    mirror_root: &Path,
) -> Result<Vec<String>, ContainmentGap> {
    let acceptable = crate::owned_run::absolute_normal(&runtime.node_path)
        && crate::owned_run::absolute_normal(&runtime.cli_path)
        && crate::owned_run::absolute_normal(mirror_root);
    if !acceptable {
        return Err(ContainmentGap::ProtocolViolation);
    }
    let node = runtime
        .node_path
        .to_str()
        .ok_or(ContainmentGap::ProtocolViolation)?;
    let cli = runtime
        .cli_path
        .to_str()
        .ok_or(ContainmentGap::ProtocolViolation)?;
    let mirror = mirror_root
        .to_str()
        .ok_or(ContainmentGap::ProtocolViolation)?;
    // 只有这三条 argv；`--continuous`、登录/配对与任意 flag 都不在此。
    let mut argv = vec![node.to_owned(), cli.to_owned()];
    match operation {
        RunOperation::RunOnce => {
            argv.push("sync".to_owned());
            argv.push("--path".to_owned());
            argv.push(mirror.to_owned());
        }
        RunOperation::InspectConfig => {
            argv.push("sync-config".to_owned());
            argv.push("--path".to_owned());
            argv.push(mirror.to_owned());
            argv.push("--json".to_owned());
        }
        // ADR-025 D4：除同步模式外不开放任何配置修改。
        RunOperation::SetMode(mode) => {
            argv.push("sync-config".to_owned());
            argv.push("--path".to_owned());
            argv.push(mirror.to_owned());
            argv.push("--mode".to_owned());
            argv.push(mode.cli_value().to_owned());
            argv.push("--json".to_owned());
        }
    }
    Ok(argv)
}

/// 封闭 argv 的域分隔摘要；owner 经 IPC 只送摘要，launcher 用自己的固定 pins 重算后比对。
///
/// 可执行路径与镜像路径永远不过 IPC：摘要不同即拒绝启动，两边的 pins 谁漂移都藏不住。
#[must_use]
pub fn argv_digest(argv: &[String]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"studiolo.sync-transport.argv.v1\0");
    for item in argv {
        hash.update((item.len() as u64).to_be_bytes());
        hash.update(item.as_bytes());
    }
    hash.finalize().into()
}

/// 运行环境必须先清空再只加入批准变量；本函数拒绝任何会改变解析行为的变量。
#[must_use]
pub fn environment_acceptable(names: &[&str]) -> bool {
    const FORBIDDEN: [&str; 6] = [
        "OBSIDIAN_AUTH_TOKEN",
        "NODE_OPTIONS",
        "NODE_PATH",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "HOME",
    ];
    names
        .iter()
        .all(|name| !FORBIDDEN.contains(name) && !name.is_empty())
}
