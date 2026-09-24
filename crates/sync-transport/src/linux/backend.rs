//! 已核验后端与真实 adapter：每个方法都是一次 launcher IPC 往返（ADR-025 D3）。
//!
//! 本文件不做 IO：socket 连接与收发只在 `client.rs`（本 crate 唯一的 socket IO 文件）。
//! 这里只组请求帧、按请求种类核对应答变体并映射成 owner 侧类型（`mapping.rs`）；
//! 应答变体不符或 nonce 不回显是 `ProtocolViolation`，身份字段不一致是 `IdentityMismatch`，不猜。

mod mapping;
mod verify;

pub use verify::{verify_backend, verify_backend_at};

use super::cgroup::CgroupEvents;
use super::launcher::{LaunchAcknowledgement, LaunchRequest, ReleaseToken};
use super::protocol::{ContainerFrame, Empty, LaunchFrame, Request, Response, StopFrame};
use super::{ContainmentBackend, ContainmentGap, IdentityMatch, client, service};
use crate::owned_run::{ContainerIdentity, ExitObservation, ResourceCounts, RunId};
use std::path::{Path, PathBuf};

/// 已实测核验的受控 Linux 后端；没有公开构造器，也不能被反序列化。
///
/// 唯一的构造路径是 `verify_backend_at`：它经 launcher IPC 的 `probe` 请求拿到实测事实，
/// 逐项与批准值比对后才返回。没有 launcher 应答的机器上任何代码路径都拿不到它。
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

/// 绑定已核验后端的真实 adapter：每个方法都是一次 launcher IPC 往返。
///
/// owner 进程自己不读 cgroupfs、不起子进程；运行标识与 `ContainerIdentity` 逐字段进请求帧，
/// launcher 逐字段核对后才回事实。应答变体与请求种类不符即 `ProtocolViolation`。
#[derive(Debug)]
pub struct LinuxContainment {
    backend: VerifiedLinuxBackend,
    /// 最近一次 `issue_launch` 成功回报的容器身份；`release_once` 只为它拼放行帧。
    last: Option<ContainerIdentity>,
}

impl LinuxContainment {
    /// 只能由已核验后端构造。
    #[must_use]
    pub fn new(backend: VerifiedLinuxBackend) -> Self {
        Self {
            backend,
            last: None,
        }
    }

    /// 已核验后端的只读事实。
    #[must_use]
    pub fn backend(&self) -> &VerifiedLinuxBackend {
        &self.backend
    }

    /// 一次往返；`Refused` 已在 `client::call` 里按 `Verdict::gap` 映射。
    fn call(&self, request: Request) -> Result<Response, ContainmentGap> {
        client::call(self.backend.socket(), &request)
    }

    /// 绑定运行标识与容器身份的一次往返。
    fn bound(
        &self,
        run_id: RunId,
        container: &ContainerIdentity,
        kind: fn(ContainerFrame) -> Request,
    ) -> Result<Response, ContainmentGap> {
        self.call(kind(ContainerFrame::from_identity(run_id, container)))
    }
}

impl ContainmentBackend for LinuxContainment {
    fn boot_id(&mut self) -> Result<[u8; 16], ContainmentGap> {
        match self.call(Request::BootId(Empty {}))? {
            Response::BootId { boot_id } => Ok(boot_id),
            _ => Err(ContainmentGap::ProtocolViolation),
        }
    }

    fn issue_launch(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<LaunchAcknowledgement, ContainmentGap> {
        let (operation, mode) = LaunchFrame::split_operation(request.operation);
        // argv 正文不过 IPC：只送摘要，launcher 用自己的固定 pins 重算比对。
        let frame = LaunchFrame {
            run_id: request.run_id,
            sequence: request.sequence,
            operation,
            mode,
            nonce: request.nonce.bytes(),
            argv_digest: service::argv_digest(&request.argv),
            budget: request.budget,
            payload_uid: request.payload_uid,
            payload_gid: request.payload_gid,
        };
        let report = match self.call(Request::Launch(frame))? {
            Response::Launch(report) => report,
            _ => return Err(ContainmentGap::ProtocolViolation),
        };
        if report.nonce != request.nonce.bytes() {
            return Err(ContainmentGap::ProtocolViolation);
        }
        if report.container.run_id != request.run_id {
            return Err(ContainmentGap::IdentityMismatch);
        }
        let container = report.container.identity();
        self.last = Some(container.clone());
        Ok(mapping::acknowledgement(request.run_id, container, &report))
    }

    fn release_once(&mut self, token: ReleaseToken) -> Result<(), ContainmentGap> {
        // 放行帧的其余身份字段来自本实例记下的 launch 回报；session 不符就不是同一次屏障。
        let container = self
            .last
            .as_ref()
            .filter(|container| container.launcher_session == token.launcher_session())
            .ok_or(ContainmentGap::IdentityMismatch)?;
        let frame = ContainerFrame::from_identity(token.run_id(), container);
        match self.call(Request::Release(frame))? {
            Response::Release(_) => Ok(()),
            _ => Err(ContainmentGap::ProtocolViolation),
        }
    }

    fn observe_running(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<bool, ContainmentGap> {
        match self.bound(run_id, container, Request::Running)? {
            Response::Running { running } => Ok(running),
            _ => Err(ContainmentGap::ProtocolViolation),
        }
    }

    fn close_spawn_gate(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<(), ContainmentGap> {
        match self.bound(run_id, container, Request::Gate)? {
            Response::Gate(_) => Ok(()),
            _ => Err(ContainmentGap::ProtocolViolation),
        }
    }

    fn read_events(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<CgroupEvents, ContainmentGap> {
        match self.bound(run_id, container, Request::Events)? {
            Response::Events { populated, frozen } => Ok(CgroupEvents { populated, frozen }),
            _ => Err(ContainmentGap::ProtocolViolation),
        }
    }

    fn drain(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<ResourceCounts, ContainmentGap> {
        match self.bound(run_id, container, Request::Drain)? {
            Response::Drain {
                stdout_bytes,
                stderr_bytes,
                output_over_budget,
            } => Ok(ResourceCounts {
                stdout_bytes,
                stderr_bytes,
                output_over_budget,
            }),
            _ => Err(ContainmentGap::ProtocolViolation),
        }
    }

    fn stop(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
        grace_seconds: u32,
    ) -> Result<(), ContainmentGap> {
        let frame = StopFrame {
            container: ContainerFrame::from_identity(run_id, container),
            grace_seconds,
        };
        match self.call(Request::Stop(frame))? {
            Response::Stop(_) => Ok(()),
            _ => Err(ContainmentGap::ProtocolViolation),
        }
    }

    fn reap(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<ExitObservation, ContainmentGap> {
        match self.bound(run_id, container, Request::Reap)? {
            Response::Reap {
                exit_observed,
                exit_code,
                exit_signal,
            } => mapping::exit_observation(exit_observed, exit_code, exit_signal),
            _ => Err(ContainmentGap::ProtocolViolation),
        }
    }

    fn match_identity(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<IdentityMatch, ContainmentGap> {
        match self.bound(run_id, container, Request::Match)? {
            Response::Match { identity } => mapping::identity_match(&identity),
            _ => Err(ContainmentGap::ProtocolViolation),
        }
    }

    fn config_output(
        &mut self,
        run_id: RunId,
        container: &ContainerIdentity,
    ) -> Result<String, ContainmentGap> {
        match self.bound(run_id, container, Request::Output)? {
            Response::Output { output } => Ok(output),
            _ => Err(ContainmentGap::ProtocolViolation),
        }
    }
}
