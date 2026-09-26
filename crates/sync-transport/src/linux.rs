//! 受控 Linux 后端的能力面。
//!
//! 真实特权路径是管理员预置的 root 系统服务 `studiolo-sync-launcher serve`（ADR-025 D2）：
//! owner 经固定私有 socket（`protocol::LAUNCHER_SOCKET`）发封闭请求，launcher 用 systemd
//! transient unit 建 payload 子组、核对隔离、放行、读 cgroup 事实并回报。本文件的
//! `verify_backend` 与 `LinuxContainment` 每个方法都是那条 IPC 的一次往返（`client.rs`，
//! 本 crate 里 socket IO 的唯一位置）：socket 未置备如实报 `TrustedLauncherNotProvisioned`，
//! 探测项任一不满足按其分类报缺口。**绝不回退 PID/PGID 观察冒充 OwnedRun**，也没有任何
//! mock 能把平台标记为可用——只有 launcher 的 `probe` 实测应答能产出 `VerifiedLinuxBackend`。

pub mod cgroup;
pub mod ipc;
pub mod launcher;
pub mod protocol;
pub mod service;
pub mod stdio;

mod backend;
pub(crate) mod client;

pub use backend::{LinuxContainment, VerifiedLinuxBackend, verify_backend, verify_backend_at};

use crate::owned_run::{ContainerIdentity, ExitObservation, ResourceCounts, RunId, TransportError};
use cgroup::CgroupEvents;
use launcher::{LaunchAcknowledgement, LaunchRequest, ReleaseToken};

/// 受控隔离缺什么的固定分类；不带原始诊断、路径或 stderr。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContainmentGap {
    /// 本平台不是 Linux；第一片明确不支持。
    UnsupportedPlatform,
    /// 管理员尚未预置固定 link 的可信 launcher/service 特权。
    TrustedLauncherNotProvisioned,
    /// 没有 cgroup v2。
    CgroupV2Missing,
    /// 没有取得子组委派。
    DelegationMissing,
    /// 需要的控制器未启用。
    ControllerMissing,
    /// 没有覆盖整组后代的 kill 能力。
    KillCapabilityMissing,
    /// 缺 mount/PID 等 namespace 隔离。
    NamespaceIsolationMissing,
    /// 缺专用 payload 身份隔离。
    DedicatedIdentityMissing,
    /// 缺私有状态目录的磁盘 quota。
    DiskQuotaMissing,
    /// 实际资源配置不满足预算。
    BudgetUnsupported,
    /// 与 manager 的连接不可用。
    ManagerUnavailable,
    /// 容器身份核对失败。
    IdentityMismatch,
    /// IPC 协议或数据形状不合法。
    ProtocolViolation,
    /// 帧超过硬上限。
    FrameTooLarge,
    /// 对端凭据不是批准的 payload 身份。
    PeerRejected,
}

impl ContainmentGap {
    /// 启动请求发出**之前**的失败：一律是「不支持受控隔离」，不启动第三方程序。
    #[must_use]
    pub fn before_start(self) -> TransportError {
        TransportError::UnsupportedContainment
    }

    /// 启动请求已经发出**之后**的失败：保持未知，保留原槽，不取 token、不启新运行。
    #[must_use]
    pub fn after_start(self) -> TransportError {
        TransportError::ExitUnconfirmed
    }
}

/// 重开时对旧容器的身份核对结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityMatch {
    /// 同一 boot、同一 unit、同一 InvocationID、同一子组对象。
    Exact,
    /// 读不到该 unit；这不是静止证明。
    UnitMissing,
    /// 同名 unit 但 InvocationID 不同；已被替换。
    UnitReplaced,
    /// 同名 cgroup 但对象身份不同；典型 ABA。
    CgroupReused,
    /// boot 已改变；旧 boot 的进程必然不存在，但原退出结果仍然未知。
    BootChanged,
}

/// 受控容器的全部 OS 能力面；不导出 raw FD、通用 exec 或写回调。
pub trait ContainmentBackend {
    /// 当前 boot 标识。
    ///
    /// # 错误
    /// manager 不可用时返回 `ManagerUnavailable`。
    fn boot_id(&mut self) -> Result<[u8; 16], ContainmentGap>;

    /// 请求可信 launcher 在屏障内建立 payload 子组并返回真实身份。
    ///
    /// # 错误
    /// 任何隔离、预算或协议缺口按其分类返回。
    fn issue_launch(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<LaunchAcknowledgement, ContainmentGap>;

    /// 消费一次放行；协议只接受一次，重复请求不得产生第二次执行。
    ///
    /// # 错误
    /// 身份不符返回 `IdentityMismatch`。
    fn release_once(&mut self, token: ReleaseToken) -> Result<(), ContainmentGap>;

    /// 观察执行是否真的开始。
    ///
    /// 运行标识与容器身份一起进请求帧（`ContainerFrame`），launcher 逐字段核对；下同。
    ///
    /// # 错误
    /// 身份不符返回 `IdentityMismatch`。
    fn observe_running(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<bool, ContainmentGap>;

    /// 关闭一切继续创建 payload 的入口。
    ///
    /// # 错误
    /// 无法确证入口已关闭时返回 `IdentityMismatch`。
    fn close_spawn_gate(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<(), ContainmentGap>;

    /// 读取同一受控 payload 子组的 `cgroup.events`。
    ///
    /// # 错误
    /// 身份不符返回 `IdentityMismatch`。
    fn read_events(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<CgroupEvents, ContainmentGap>;

    /// 取得当前有界排空计数。
    ///
    /// # 错误
    /// 身份不符返回 `IdentityMismatch`。
    fn drain(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<ResourceCounts, ContainmentGap>;

    /// 只停止绑定的子组：先 TERM 有界 grace，再对同一组执行覆盖全后代的强停。
    ///
    /// # 错误
    /// 身份不符返回 `IdentityMismatch`。
    fn stop(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
        grace_seconds: u32,
    ) -> Result<(), ContainmentGap>;

    /// 取得真实等待结果；取不到就是 `NotObserved`。
    ///
    /// # 错误
    /// 身份不符返回 `IdentityMismatch`。
    fn reap(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<ExitObservation, ContainmentGap>;

    /// 重开时核对旧容器身份。
    ///
    /// # 错误
    /// manager 不可用时返回 `ManagerUnavailable`。
    fn match_identity(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<IdentityMatch, ContainmentGap>;

    /// 已 reap 的 InspectConfig／SetMode 运行的 stdout 正文（UTF-8，≤ `config_json_bytes`）。
    ///
    /// 只有 launcher 才持有 payload 的输出文件；默认实现如实报协议缺口，假后端按需覆盖。
    ///
    /// # 错误
    /// 未 reap、超预算或非 UTF-8 返回 `BudgetUnsupported`；身份不符返回 `IdentityMismatch`。
    fn config_output(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<String, ContainmentGap> {
        let _ = (run_id, container);
        Err(ContainmentGap::ProtocolViolation)
    }
}
