//! launcher → owner 的应答与 payload 垫片的报告帧；纯 DTO，无 IO。
//!
//! 每个应答字段都必须来自 launcher 的**实测**（`systemctl show`、cgroupfs、statfs、
//! 垫片报告），不许凭「已请求」填 true；这是 `IsolationReport` 七项的唯一来源。

use crate::owned_run::RunDiagnostics;
use serde::{Deserialize, Serialize};

/// launcher 拒绝的固定分类；不带原始诊断。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// 请求形状或协议版本不合法。
    Protocol,
    /// 对端凭据不是批准的 owner。
    Peer,
    /// 容器身份逐字段核对失败。
    IdentityMismatch,
    /// 上一 session 仍有存活进程，或已有活动 session。
    Busy,
    /// systemd manager 或 cgroupfs 不可用。
    ManagerUnavailable,
    /// 预算或实际限额不可核验。
    Budget,
    /// 管理员尚未完成置备（配置缺失、loop 镜像未挂、身份不存在）。
    NotProvisioned,
    /// argv 摘要与 launcher 自算不一致。
    ArgvMismatch,
}

impl Verdict {
    /// 映射到 owner 侧的固定缺口分类。
    #[must_use]
    pub fn gap(self) -> crate::linux::ContainmentGap {
        use crate::linux::ContainmentGap as Gap;
        match self {
            Self::Protocol | Self::ArgvMismatch => Gap::ProtocolViolation,
            Self::Peer => Gap::PeerRejected,
            Self::IdentityMismatch | Self::Busy => Gap::IdentityMismatch,
            Self::ManagerUnavailable => Gap::ManagerUnavailable,
            Self::Budget => Gap::BudgetUnsupported,
            Self::NotProvisioned => Gap::TrustedLauncherNotProvisioned,
        }
    }
}

/// `probe` 应答：全部由 launcher 实测填写。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeReport {
    /// 回显的 nonce。
    pub nonce: [u8; 32],
    /// 协议版本。
    pub protocol_version: u32,
    /// `systemctl --version` 主版本。
    pub systemd_version: u32,
    /// cgroup v2 是否挂载在 `/sys/fs/cgroup`。
    pub cgroup_v2: bool,
    /// 镜像根所在文件系统是否由固定 loop 镜像文件支撑。
    pub loop_quota: bool,
    /// 专用 payload 身份是否存在且无补充组。
    pub dedicated_identity: bool,
    /// 实测的 payload UID。
    pub payload_uid: u32,
    /// 实测的 payload GID。
    pub payload_gid: u32,
    /// 当前 boot 标识。
    pub boot_id: [u8; 16],
}

/// `launch` 应答：屏障已建立、payload 已被垫片挡住时的全部实测事实。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchReport {
    /// 回显的 nonce。
    pub nonce: [u8; 32],
    /// 实测容器身份。
    pub container: super::ContainerFrame,
    /// 实测子组限额。
    pub limits: LimitsFrame,
    /// 七项隔离核对结果。
    pub isolation: IsolationFrame,
    /// 仅诊断。
    pub diagnostics: RunDiagnostics,
}

/// `cgroup.*` 实测限额的线上形状；`None` 即内核报告 `max`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsFrame {
    /// `memory.max`。
    pub memory_max: Option<u64>,
    /// `memory.swap.max`。
    pub memory_swap_max: Option<u64>,
    /// `pids.max`。
    pub pids_max: Option<u64>,
    /// `cpu.max` 的 `(quota, period)`。
    pub cpu_max: Option<(u64, u64)>,
}

/// 七项隔离核对；与 `launcher::IsolationReport` 一一对应。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IsolationFrame {
    /// 独立 mount 视图且不含 canonical／writer-data／Bridge DB／外 stage／宿主 home。
    pub mount_namespace: bool,
    /// 独立 PID 视图。
    pub pid_namespace: bool,
    /// 已降为专用身份且无补充组。
    pub dedicated_identity: bool,
    /// capabilities 已清空。
    pub dropped_capabilities: bool,
    /// payload 无 cgroupfs 写权限、够不到 launcher socket。
    pub payload_cannot_write_cgroup: bool,
    /// 垫片报回的 (dev,ino) 与批准 pins 逐项相同。
    pub final_view_identity_checked: bool,
    /// 镜像根落在固定 loop 镜像文件上。
    pub disk_quota_enforced: bool,
}

/// launcher → owner 的全部应答；`Refused` 可以回给任何请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    /// 请求被拒；`verdict` 是唯一原因字段。
    Refused {
        /// 固定分类。
        verdict: Verdict,
    },
    /// 探测结果。
    Probe(ProbeReport),
    /// boot 标识。
    BootId {
        /// 当前 boot 标识。
        boot_id: [u8; 16],
    },
    /// 屏障已建立。
    Launch(LaunchReport),
    /// 已放行。
    Release(super::Empty),
    /// 执行是否开始。
    Running {
        /// payload 是否已 exec 固定程序。
        running: bool,
    },
    /// 入口已关闭。
    Gate(super::Empty),
    /// 子组事件。
    Events {
        /// 本组及后代是否仍有存活进程。
        populated: bool,
        /// 是否冻结。
        frozen: bool,
    },
    /// 排空计数。
    Drain {
        /// stdout 已排空字节。
        stdout_bytes: u64,
        /// stderr 已排空字节。
        stderr_bytes: u64,
        /// 是否触及限额。
        output_over_budget: bool,
    },
    /// 停止请求已下达。
    Stop(super::Empty),
    /// 真实等待结果。
    Reap {
        /// 是否取得等待结果。
        exit_observed: bool,
        /// `ExecMainStatus`（正常退出）。
        exit_code: Option<i32>,
        /// `ExecMainStatus`（信号终止）。
        exit_signal: Option<i32>,
    },
    /// 身份核对结论。
    Match {
        /// `exact`／`unit_missing`／`unit_replaced`／`cgroup_reused`／`boot_changed`。
        identity: String,
    },
    /// stdout 正文。
    Output {
        /// UTF-8 正文；超预算或非 UTF-8 时 launcher 改回 `Refused(Budget)`。
        output: String,
    },
}

/// 垫片经报告 FIFO 送回 launcher 的唯一一帧：最终 mount 视图内的 (dev,ino) 事实。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadReport {
    /// 固定为 `"payload_report"`。
    pub kind: PayloadReportKind,
    /// 镜像根 `(dev, ino)`。
    pub mirror_identity: (u64, u64),
    /// headless 配置根 `(dev, ino)`。
    pub config_identity: (u64, u64),
    /// 固定 Node 可执行文件 `(dev, ino)`。
    pub node_identity: (u64, u64),
    /// 固定 `cli.js` `(dev, ino)`。
    pub cli_identity: (u64, u64),
    /// 垫片实际运行 UID。
    pub uid: u32,
    /// 垫片实际运行 GID。
    pub gid: u32,
}

/// `PayloadReport.kind` 的单值枚举。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadReportKind {
    /// 唯一值。
    PayloadReport,
}
