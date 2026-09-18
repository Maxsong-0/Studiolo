//! 受控 Linux 后端的能力面。
//!
//! **平台前置未满足**：本仓库尚未由管理员预置可信 launcher/service 特权，也没有隔离
//! 测试机，因此 `VerifiedLinuxBackend` 在任何平台上都还构造不出来。macOS 与缺权限的
//! Linux 一律返回 `UnsupportedContainment`；**绝不回退 PID/PGID 观察冒充 OwnedRun**，
//! 也没有任何 mock 能把平台标记为可用。

pub mod cgroup;
pub mod ipc;
pub mod launcher;
pub mod service;
pub mod stdio;

use crate::owned_run::{
    ApprovedTransport, ContainerIdentity, ExitObservation, ResourceCounts, TransportError,
};
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
    /// # Errors
    /// manager 不可用时返回 `ManagerUnavailable`。
    fn boot_id(&mut self) -> Result<[u8; 16], ContainmentGap>;

    /// 请求可信 launcher 在屏障内建立 payload 子组并返回真实身份。
    ///
    /// # Errors
    /// 任何隔离、预算或协议缺口按其分类返回。
    fn issue_launch(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<LaunchAcknowledgement, ContainmentGap>;

    /// 消费一次放行；协议只接受一次，重复请求不得产生第二次执行。
    ///
    /// # Errors
    /// 身份不符返回 `IdentityMismatch`。
    fn release_once(&mut self, token: ReleaseToken) -> Result<(), ContainmentGap>;

    /// 观察执行是否真的开始。
    ///
    /// # Errors
    /// 身份不符返回 `IdentityMismatch`。
    fn observe_running(&mut self, container: &ContainerIdentity) -> Result<bool, ContainmentGap>;

    /// 关闭一切继续创建 payload 的入口。
    ///
    /// # Errors
    /// 无法确证入口已关闭时返回 `IdentityMismatch`。
    fn close_spawn_gate(&mut self, container: &ContainerIdentity) -> Result<(), ContainmentGap>;

    /// 读取同一受控 payload 子组的 `cgroup.events`。
    ///
    /// # Errors
    /// 身份不符返回 `IdentityMismatch`。
    fn read_events(
        &mut self,
        container: &ContainerIdentity,
    ) -> Result<CgroupEvents, ContainmentGap>;

    /// 取得当前有界排空计数。
    ///
    /// # Errors
    /// 身份不符返回 `IdentityMismatch`。
    fn drain(&mut self, container: &ContainerIdentity) -> Result<ResourceCounts, ContainmentGap>;

    /// 只停止绑定的子组：先 TERM 有界 grace，再对同一组执行覆盖全后代的强停。
    ///
    /// # Errors
    /// 身份不符返回 `IdentityMismatch`。
    fn stop(
        &mut self,
        container: &ContainerIdentity,
        grace_seconds: u32,
    ) -> Result<(), ContainmentGap>;

    /// 取得真实等待结果；取不到就是 `NotObserved`。
    ///
    /// # Errors
    /// 身份不符返回 `IdentityMismatch`。
    fn reap(&mut self, container: &ContainerIdentity) -> Result<ExitObservation, ContainmentGap>;

    /// 重开时核对旧容器身份。
    ///
    /// # Errors
    /// manager 不可用时返回 `ManagerUnavailable`。
    fn match_identity(
        &mut self,
        container: &ContainerIdentity,
    ) -> Result<IdentityMatch, ContainmentGap>;
}

/// 已实测核验的受控 Linux 后端；没有公开构造器，也不能被反序列化。
///
/// 目前**任何平台都拿不到它**：仓库尚未具备管理员预置的可信 launcher/service 特权。
///
/// ```compile_fail
/// let backend: studiolo_sync_transport::linux::VerifiedLinuxBackend =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct VerifiedLinuxBackend {
    service_unit: String,
    payload_uid: u32,
    payload_gid: u32,
}

impl VerifiedLinuxBackend {
    /// 已核验的精确 unit 名。
    #[must_use]
    pub fn service_unit(&self) -> &str {
        &self.service_unit
    }

    /// 已核验的专用 payload 身份。
    #[must_use]
    pub fn payload_identity(&self) -> (u32, u32) {
        (self.payload_uid, self.payload_gid)
    }
}

/// 首次检查受控隔离前置；缺任何一项都不启动第三方程序。
///
/// 当前实现在所有平台都返回缺口：非 Linux 是 `UnsupportedPlatform`，Linux 是
/// `TrustedLauncherNotProvisioned`。这不是占位符，而是如实报告本机确实没有该特权 ——
/// 在管理员预置可信 launcher/service 并在隔离 Linux 上实测之前，这里不得返回 `Ok`。
///
/// # Errors
/// 永远返回缺口分类；平台「受支持」状态留待实测。
pub fn verify_backend(
    approval: &dyn ApprovedTransport,
) -> Result<VerifiedLinuxBackend, ContainmentGap> {
    approval
        .revalidate()
        .map_err(|_| ContainmentGap::IdentityMismatch)?;
    let backend = approval.backend();
    if !service::unit_name_acceptable(&backend.service_unit) {
        return Err(ContainmentGap::ProtocolViolation);
    }
    if !cfg!(target_os = "linux") {
        return Err(ContainmentGap::UnsupportedPlatform);
    }
    // 真实探测（cgroup v2 挂载、委派、控制器、kill 能力、namespace、专用身份、磁盘 quota）
    // 必须经由管理员预置的可信 launcher 完成；本仓库还没有那条特权路径，所以如实报缺口，
    // 绝不用 PID/PGID 观察或任何 mock 把平台标记为可用。
    Err(ContainmentGap::TrustedLauncherNotProvisioned)
}

/// 绑定已核验后端的真实 adapter。
///
/// 每个方法的真实 OS 调用（systemd 接口、cgroup FD、launcher IPC）必须在管理员预置
/// 可信 launcher/service 并在隔离 Linux 上实测之后补齐。在那之前，本类型连构造都做不到
/// （它需要一个 `VerifiedLinuxBackend`），因此这里如实报告缺口而不是给一个会「看起来能跑」
/// 的假实现 —— 假实现正是本设计明令禁止的 PID/PGID 降级。
#[derive(Debug)]
pub struct LinuxContainment {
    backend: VerifiedLinuxBackend,
}

impl LinuxContainment {
    /// 只能由已核验后端构造。
    #[must_use]
    pub fn new(backend: VerifiedLinuxBackend) -> Self {
        Self { backend }
    }

    /// 已核验后端的只读事实。
    #[must_use]
    pub fn backend(&self) -> &VerifiedLinuxBackend {
        &self.backend
    }

    fn gap(&self) -> ContainmentGap {
        ContainmentGap::TrustedLauncherNotProvisioned
    }
}

impl ContainmentBackend for LinuxContainment {
    fn boot_id(&mut self) -> Result<[u8; 16], ContainmentGap> {
        Err(self.gap())
    }

    fn issue_launch(
        &mut self,
        _request: &LaunchRequest,
    ) -> Result<LaunchAcknowledgement, ContainmentGap> {
        Err(self.gap())
    }

    fn release_once(&mut self, _token: ReleaseToken) -> Result<(), ContainmentGap> {
        Err(self.gap())
    }

    fn observe_running(&mut self, _container: &ContainerIdentity) -> Result<bool, ContainmentGap> {
        Err(self.gap())
    }

    fn close_spawn_gate(&mut self, _container: &ContainerIdentity) -> Result<(), ContainmentGap> {
        Err(self.gap())
    }

    fn read_events(
        &mut self,
        _container: &ContainerIdentity,
    ) -> Result<CgroupEvents, ContainmentGap> {
        Err(self.gap())
    }

    fn drain(&mut self, _container: &ContainerIdentity) -> Result<ResourceCounts, ContainmentGap> {
        Err(self.gap())
    }

    fn stop(
        &mut self,
        _container: &ContainerIdentity,
        _grace_seconds: u32,
    ) -> Result<(), ContainmentGap> {
        Err(self.gap())
    }

    fn reap(&mut self, _container: &ContainerIdentity) -> Result<ExitObservation, ContainmentGap> {
        Err(self.gap())
    }

    fn match_identity(
        &mut self,
        _container: &ContainerIdentity,
    ) -> Result<IdentityMatch, ContainmentGap> {
        Err(self.gap())
    }
}
