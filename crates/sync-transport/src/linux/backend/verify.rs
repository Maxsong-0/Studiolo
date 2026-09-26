//! `verify_backend`／`verify_backend_at`：唯一能产出 `VerifiedLinuxBackend` 的路径，
//! 探测事实只来自 launcher 的 `probe` 应答。从 `backend.rs` 拆出只为压住单文件行数。

use super::super::protocol::{self, Request, Response};
use super::super::{ContainmentGap, client, service};
use super::{VerifiedLinuxBackend, mapping};
use crate::owned_run::{ApprovedTransport, random};
use std::path::Path;

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
    // 探测事实只来自 launcher 的 `probe` 应答；探测不到就是缺口，绝不用 PID/PGID 观察或任何
    // mock 把平台标记为可用。
    let mut nonce = [0u8; 32];
    random(&mut nonce).map_err(|_| ContainmentGap::ManagerUnavailable)?;
    let report = match client::call(socket, &Request::Probe { nonce })? {
        Response::Probe(report) => report,
        _ => return Err(ContainmentGap::ProtocolViolation),
    };
    mapping::probe_acceptable(&report, nonce, &backend)?;
    Ok(VerifiedLinuxBackend {
        service_unit: backend.service_unit,
        payload_uid: backend.payload_uid,
        payload_gid: backend.payload_gid,
        socket: socket.to_path_buf(),
    })
}
