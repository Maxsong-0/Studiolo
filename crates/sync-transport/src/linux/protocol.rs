//! owner ↔ launcher IPC 与 payload 垫片报告的**唯一**线上契约（ADR-025 D2／D3）。
//!
//! 纯 DTO：本文件不开 socket、不读文件。帧格式复用 `ipc::{encode_frame,decode_frame}`，
//! 顶层键闭集由 `ipc::ALLOWED_KEYS` 守住，本文件的 `ALL_KEYS` 必须是它的子集
//! （W1 在 `tests/owned_run_private/adapters.rs` 加用例锁住）。可执行路径、镜像路径、
//! argv 正文、token 永远不进任何帧：argv 只送 `argv_digest`，launcher 用自己的固定 pins
//! 重算后比对。应答与垫片报告见 `protocol/response.rs`。

mod response;

pub use response::{
    IsolationFrame, LaunchReport, LimitsFrame, PayloadReport, PayloadReportKind, ProbeReport,
    Response, Verdict,
};

use crate::owned_run::{ContainerIdentity, RunBudget, RunId, RunOperation, SyncMode};
use serde::{Deserialize, Serialize};

/// launcher 私有 socket 的固定路径；owner 不接受任何别的端点。
pub const LAUNCHER_SOCKET: &str = "/run/studiolo-sync-launcher/launcher.sock";
/// 本协议版本；两端不一致即 `Verdict::Protocol`。
pub const PROTOCOL_VERSION: u32 = 1;

/// 空载荷；带 `deny_unknown_fields` 的占位，让「没有字段」也是闭集。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

/// 容器绑定请求共用的身份字段；launcher 逐字段核对，任何一项不符即 `IdentityMismatch`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerFrame {
    /// 运行标识。
    pub run_id: RunId,
    /// 请求启动时固定的 boot 标识。
    pub boot_id: [u8; 16],
    /// 精确 unit 名。
    pub unit_name: String,
    /// manager 侧 InvocationID。
    pub invocation_id: [u8; 16],
    /// payload 子组路径。
    pub payload_cgroup: String,
    /// payload 子组 `(dev, ino)`。
    pub cgroup_identity: (u64, u64),
    /// 一次性 launcher 会话。
    pub launcher_session: [u8; 16],
}

impl ContainerFrame {
    /// 由 owner 持久记录里的容器身份构造。
    #[must_use]
    pub fn from_identity(run_id: RunId, container: &ContainerIdentity) -> Self {
        Self {
            run_id,
            boot_id: container.boot_id,
            unit_name: container.unit_name.clone(),
            invocation_id: container.invocation_id,
            payload_cgroup: container.payload_cgroup.clone(),
            cgroup_identity: container.cgroup_identity,
            launcher_session: container.launcher_session,
        }
    }

    /// 反向投影成 owner 侧的容器身份。
    #[must_use]
    pub fn identity(&self) -> ContainerIdentity {
        ContainerIdentity {
            boot_id: self.boot_id,
            unit_name: self.unit_name.clone(),
            invocation_id: self.invocation_id,
            payload_cgroup: self.payload_cgroup.clone(),
            cgroup_identity: self.cgroup_identity,
            launcher_session: self.launcher_session,
        }
    }
}

/// `launch` 请求：不含 argv 正文，只含操作、模式、预算与 argv 摘要。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchFrame {
    /// 运行标识。
    pub run_id: RunId,
    /// 运行序号；unit 名由它与 run_id 前 8 位十六进制拼成。
    pub sequence: u64,
    /// 固定操作名（`inspect_config`／`run_once`／`set_mode`）。
    pub operation: String,
    /// 只有 `set_mode` 才有的目标模式。
    pub mode: Option<SyncMode>,
    /// 本次握手 nonce；launcher 原样回显。
    pub nonce: [u8; 32],
    /// `service::argv_digest` 的结果。
    pub argv_digest: [u8; 32],
    /// 运行预算；launcher 据此设置 transient unit 属性并回读核对。
    pub budget: RunBudget,
    /// 期望的专用 payload UID。
    pub payload_uid: u32,
    /// 期望的专用 payload GID。
    pub payload_gid: u32,
}

impl LaunchFrame {
    /// 把 `RunOperation` 拆成固定操作名与可选模式。
    #[must_use]
    pub fn split_operation(operation: RunOperation) -> (String, Option<SyncMode>) {
        let mode = match operation {
            RunOperation::SetMode(mode) => Some(mode),
            RunOperation::InspectConfig | RunOperation::RunOnce => None,
        };
        (operation.name().to_owned(), mode)
    }
}

/// `stop` 请求：容器身份加有界 TERM 宽限。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopFrame {
    /// 容器身份。
    pub container: ContainerFrame,
    /// TERM 后的宽限秒数；到期用 `KillMode=control-group` 强停整组。
    pub grace_seconds: u32,
}

/// owner → launcher 的全部请求；`kind` 是外层标签。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    /// 探测平台前置（systemd 版本、cgroup v2、loop quota、专用身份）。
    Probe {
        /// 本次探测的 nonce。
        nonce: [u8; 32],
    },
    /// 当前 boot 标识。
    BootId(Empty),
    /// 建立 payload 子组与屏障，不放行。
    Launch(LaunchFrame),
    /// 一次性放行；同一 session 第二次请求一律拒绝。
    Release(ContainerFrame),
    /// 执行是否真的开始。
    Running(ContainerFrame),
    /// 关闭继续创建 payload 的入口（作废本 session 的放行能力）。
    Gate(ContainerFrame),
    /// 读同一子组的 `cgroup.events`。
    Events(ContainerFrame),
    /// 当前有界排空计数。
    Drain(ContainerFrame),
    /// 只停绑定子组。
    Stop(StopFrame),
    /// 真实等待结果。
    Reap(ContainerFrame),
    /// 重开时核对旧容器身份。
    Match(ContainerFrame),
    /// 已 reap 的 InspectConfig／SetMode 的 stdout 正文（UTF-8，≤ `config_json_bytes`）。
    Output(ContainerFrame),
}

/// 本协议所有 DTO 用到的顶层键；必须是 `ipc::ALLOWED_KEYS` 的子集。
pub const ALL_KEYS: [&str; 44] = [
    "kind",
    "nonce",
    "run_id",
    "sequence",
    "operation",
    "mode",
    "argv_digest",
    "budget",
    "payload_uid",
    "payload_gid",
    "boot_id",
    "unit_name",
    "invocation_id",
    "payload_cgroup",
    "cgroup_identity",
    "launcher_session",
    "container",
    "grace_seconds",
    "verdict",
    "protocol_version",
    "systemd_version",
    "cgroup_v2",
    "loop_quota",
    "dedicated_identity",
    "limits",
    "isolation",
    "diagnostics",
    "running",
    "populated",
    "frozen",
    "stdout_bytes",
    "stderr_bytes",
    "output_over_budget",
    "exit_observed",
    "exit_code",
    "exit_signal",
    "identity",
    "output",
    "mirror_identity",
    "config_identity",
    "node_identity",
    "cli_identity",
    "uid",
    "gid",
];
