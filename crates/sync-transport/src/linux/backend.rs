//! 已核验后端与真实 adapter：每个方法都是一次 launcher IPC 往返（ADR-025 D3）。
//!
//! 本文件不做 IO：socket 连接与收发只在 `client.rs`（W1 新建，唯一豁免的 IO 文件）。
//! 在 W1 接上之前，这里如实报 `TrustedLauncherNotProvisioned`，绝不给「看起来能跑」的假实现。

use super::cgroup::CgroupEvents;
use super::launcher::{LaunchAcknowledgement, LaunchRequest, ReleaseToken};
use super::{ContainmentBackend, ContainmentGap, IdentityMatch, protocol, service};
use crate::owned_run::{ApprovedTransport, ContainerIdentity, ExitObservation, ResourceCounts};
use std::path::{Path, PathBuf};

/// 已实测核验的受控 Linux 后端；没有公开构造器，也不能被反序列化。
///
/// 唯一的构造路径是 `verify_backend_at`：它经 launcher IPC 的 `probe` 请求拿到实测事实，
/// 逐项与批准值比对后才返回。W1 接上 `client.rs` 之前任何平台都拿不到它。
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
    socket: PathBuf,
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

    /// 探测时用过的 launcher socket；后续每个 IPC 调用都连同一个端点。
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

/// 首次检查受控隔离前置；缺任何一项都不启动第三方程序。
///
/// 生产固定连 `protocol::LAUNCHER_SOCKET`；契约测试用 `verify_backend_at` 指向临时 socket
/// 上的假 launcher。探测事实全部来自 launcher 的 `probe` 应答，本函数不读 cgroupfs、不起子进程。
///
/// # 错误
/// 非 Linux 返回 `UnsupportedPlatform`；launcher 未置备或探测项任一不满足返回对应缺口。
pub fn verify_backend(
    approval: &dyn ApprovedTransport,
) -> Result<VerifiedLinuxBackend, ContainmentGap> {
    verify_backend_at(approval, Path::new(protocol::LAUNCHER_SOCKET))
}

/// 同 `verify_backend`，但显式指定 launcher socket；生产只经 `verify_backend` 调用。
///
/// # 错误
/// 同 `verify_backend`。
pub fn verify_backend_at(
    approval: &dyn ApprovedTransport,
    socket: &Path,
) -> Result<VerifiedLinuxBackend, ContainmentGap> {
    approval
        .revalidate()
        .map_err(|_| ContainmentGap::IdentityMismatch)?;
    let backend = approval.backend();
    if !service::unit_name_acceptable(&backend.service_unit)
        || !crate::owned_run::absolute_normal(socket)
    {
        return Err(ContainmentGap::ProtocolViolation);
    }
    if !cfg!(target_os = "linux") {
        return Err(ContainmentGap::UnsupportedPlatform);
    }
    // B 轮 W1 实现：连接 socket → `Request::Probe` → 核对 `ProbeReport`（protocol_version、
    // cgroup_v2、loop_quota、dedicated_identity、payload_uid/gid 与批准值相等）→ 构造
    // `VerifiedLinuxBackend { service_unit, payload_uid, payload_gid, socket }`。
    // 探测不到就是缺口，绝不用 PID/PGID 观察或任何 mock 把平台标记为可用。
    let _ = (backend, socket);
    Err(ContainmentGap::TrustedLauncherNotProvisioned)
}

/// 绑定已核验后端的真实 adapter：每个方法都是一次 launcher IPC 往返。
///
/// owner 进程自己不读 cgroupfs、不起子进程；`ContainerIdentity` 逐字段进请求帧，
/// launcher 逐字段核对后才回事实。W1 接上 `client.rs` 之前每个方法如实报缺口。
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

    // B 轮 W1 实现：换成 `client::call(self.backend.socket(), &Request)`，按 `Response` 变体
    // 映射；`Refused { verdict }` 走 `verdict.gap()`。
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

    fn config_output(&mut self, _container: &ContainerIdentity) -> Result<String, ContainmentGap> {
        Err(self.gap())
    }
}
