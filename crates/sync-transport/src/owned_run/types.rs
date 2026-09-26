//! 纯 DTO：阶段、观察与身份。这里没有任何 OS、进程或文件能力。
//!
//! 所有「证明」类字段都必须由 adapter 实测填写；本模块不会把缺证据当成好结果。

use serde::{Deserialize, Serialize};
use std::fmt;

/// 固定 16 字节随机运行标识；不可从序号或 PID 推导。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId([u8; 16]);

impl RunId {
    /// 从已取得的随机字节构造。
    #[must_use]
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// 返回原始字节。
    #[must_use]
    pub fn as_bytes(&self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// headless 镜像的同步模式；P1 只开放这两个值（ADR-025 D4）。
///
/// `MirrorRemote` 只下载、永不上传，是引导期（P4～P5）的唯一模式；
/// `Bidirectional` 属 P6，由管理员改审批文档后才会被驱动器申请。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// 镜像只跟随远端；本地改动会被回滚，崩溃后自愈且不可能上传。
    MirrorRemote,
    /// 双向；出站改动会上传到远端。
    Bidirectional,
}

impl SyncMode {
    /// headless `sync-config --mode` 接受的字面量。
    #[must_use]
    pub fn cli_value(self) -> &'static str {
        match self {
            Self::MirrorRemote => "mirror-remote",
            Self::Bidirectional => "bidirectional",
        }
    }

    /// 从 headless `sync-config --json` 的 `syncMode` 字段解析；其他值一律不认。
    #[must_use]
    pub fn from_cli_value(value: &str) -> Option<Self> {
        match value {
            "mirror-remote" => Some(Self::MirrorRemote),
            "bidirectional" => Some(Self::Bidirectional),
            _ => None,
        }
    }
}

/// 开放的三种操作；其余动作（登录、配对、continuous、任意配置项）不在 runtime API 内。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOperation {
    /// 同一容器协议下的只读配置查询；不提供任何修改入口。
    InspectConfig,
    /// 单次同步；不是 Continuous，也不接受任意 argv。
    RunOnce,
    /// 只改镜像同步模式（`sync-config --mode`）；除此之外不开放任何配置修改。
    SetMode(SyncMode),
}

impl RunOperation {
    /// 审计与 IPC 里的固定操作名；`SetMode` 的模式另走 `mode` 字段。
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::InspectConfig => "inspect_config",
            Self::RunOnce => "run_once",
            Self::SetMode(_) => "set_mode",
        }
    }
}

/// 停止原因单调升级；一旦记录就不再降级，也不被后来的 exit 0 覆盖。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// owner 显式请求停止。
    OwnerRequested,
    /// 超过固定 wall deadline。
    DeadlineExceeded,
    /// stdout/stderr 超过字节限额；不得截断后按成功解析。
    OutputBudgetExceeded,
    /// 已发出执行请求后丢失容器身份或管理器连接。
    ContainmentLost,
}

/// 运行阶段；每一步的持久边界见 `machine`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    /// state/Intent 已可靠发布，尚无任何启动命令。
    Prepared,
    /// 尚未 LaunchIssued 就被取消；固定「未执行」终态。
    Cancelled,
    /// 已在请求启动 launcher **之前**持久。
    LaunchIssued,
    /// 屏障内可信 launcher 已建 payload 子组，owner 已固定身份并核对隔离/预算。
    Contained,
    /// 已在放行屏障**之前**持久；协议只消费一次放行。
    ExecutionIssued,
    /// 来自实际执行观察，不是「已发出请求」。
    Running,
    /// 停止原因已先持久。
    StopRequested,
    /// 继续创建 payload 的路径已关闭，且同一受控子组 `populated=0`。
    Quiesced,
    /// 固定 Outcome 已可靠保存，等待审计 ACK。
    OutcomePending,
    /// Outcome 已 ACK；可原子移入 `last_terminal`。
    Closed,
}

impl RunPhase {
    /// 是否已经发出过任何启动请求；它决定重开时能否声称「从未执行」。
    #[must_use]
    pub fn execution_may_have_started(self) -> bool {
        !matches!(self, Self::Prepared | Self::Cancelled)
    }

    /// 是否已经消费过一次放行；用于禁止第二次 spawn。
    #[must_use]
    pub fn release_consumed(self) -> bool {
        matches!(
            self,
            Self::ExecutionIssued
                | Self::Running
                | Self::StopRequested
                | Self::Quiesced
                | Self::OutcomePending
                | Self::Closed
        )
    }
}

/// 静止证据分开报告；没有实测子组空证明就是 `Unconfirmed`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quiescence {
    /// 已关闭继续 spawn 入口，且同一受控 payload 子组实测 `populated=0`。
    Confirmed,
    /// 没有取得上述证明；不得据此安装镜像或发静止能力。
    Unconfirmed,
}

/// 退出结果与静止完全独立；只有真实取得等待结果才算观察到。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitObservation {
    /// 实际回收到的退出码。
    Code(i32),
    /// 实际回收到的终止信号。
    Signal(i32),
    /// 没有取得等待结果；`MainPID=0`/`ActiveState=inactive` 都不算。
    NotObserved,
}

impl ExitObservation {
    /// 是否真的取得过进程等待结果。
    #[must_use]
    pub fn reaped(self) -> bool {
        !matches!(self, Self::NotObserved)
    }
}

/// 四项分开报告；调用方不得把其中任何一项解释为同步成功。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunObservation {
    /// 当前阶段。
    pub phase: RunPhase,
    /// 静止是否已被证明。
    pub quiescence: Quiescence,
    /// 原始退出结果。
    pub exit: ExitObservation,
    /// 已持久的停止原因。
    pub stop_reason: Option<StopReason>,
}

/// 受控容器的完整身份；PID/start-time 不在其中。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerIdentity {
    /// 本次 boot 标识；boot 变化能证明旧 boot 的进程已不存在。
    pub boot_id: [u8; 16],
    /// 精确 unit 名；owner 不接受任意 unit。
    pub unit_name: String,
    /// manager 侧 InvocationID；同名 unit 重启后必然不同。
    pub invocation_id: [u8; 16],
    /// payload 子组路径。
    pub payload_cgroup: String,
    /// payload 子组的 `(dev, ino)`；用于拒绝同名 cgroup 的 ABA 冒充。
    pub cgroup_identity: (u64, u64),
    /// 一次性 launcher 会话标识；不跨运行复用。
    pub launcher_session: [u8; 16],
}

/// 仅诊断字段；不得代替容器身份，也不授权任何 kill。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunDiagnostics {
    /// 观察到的 PID。
    pub pid: Option<i32>,
    /// 观察到的进程启动时刻。
    pub start_time: Option<u64>,
}

/// 实测的资源计数；只保存计数与固定分类，不保存正文。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceCounts {
    /// 已排空的 stdout 字节数。
    pub stdout_bytes: u64,
    /// 已排空的 stderr 字节数。
    pub stderr_bytes: u64,
    /// 是否触及输出限额。
    pub output_over_budget: bool,
}

/// 安全逻辑元数据；不含 stdout、token、原配置或任意 JSON。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunStatus {
    /// 持久提交代次。
    pub generation: u64,
    /// 下一个待分配序号；已分配序号永不复用。
    pub next_sequence: u64,
    /// 至多一个活动运行的当前观察。
    pub active: Option<ActiveSummary>,
    /// 至多一个已审计闭合摘要。
    pub last_terminal: Option<TerminalSummary>,
    /// 是否还有待补审计事件；为真时不启新运行、不发静止能力。
    pub audit_pending: bool,
}

/// 活动槽摘要。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveSummary {
    /// 运行标识。
    pub run_id: RunId,
    /// 运行序号。
    pub sequence: u64,
    /// 固定操作。
    pub operation: RunOperation,
    /// 当前观察。
    pub observation: RunObservation,
}

/// 已审计闭合摘要；完整历史在 Audit，不做无限目录扫描。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalSummary {
    /// 运行标识。
    pub run_id: RunId,
    /// 运行序号。
    pub sequence: u64,
    /// 固定操作。
    pub operation: RunOperation,
    /// 闭合时的观察四元组。
    pub observation: RunObservation,
    /// 实测资源计数。
    pub counts: ResourceCounts,
    /// 闭合时记录里的受控容器身份；未曾 Contained（如取消）即 `None`。
    /// `config_output` 只对这个身份向 launcher 取正文，绝不按名字重找 unit。
    pub container: Option<ContainerIdentity>,
}
