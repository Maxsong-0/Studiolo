//! launcher IPC 的唯一 IO 文件（ADR-025 D3）：连固定私有 socket，一问一答，有界帧。
//!
//! 本文件只做四件事：连接、写一帧、读一帧、解码。不开任何文件、不起子进程、不重试、
//! 不缓存连接：每个请求都是一次独立的连接与往返，launcher 侧按 SO_PEERCRED 逐连接核对。
//! 生产端点固定为 `protocol::LAUNCHER_SOCKET`；契约测试经 `verify_backend_at` 把同名 socket
//! 放进临时目录里的假 launcher。端点基名必须与固定常量一致，owner 不接受别的名字。

use super::ContainmentGap;
use super::ipc::{self, FRAME_MAX_BYTES};
use super::protocol::{self, Request, Response};
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// 单帧读写的固定超时；与 launcher 侧「socket 单帧读写 5 s」对齐。
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// 一次请求／应答往返。
///
/// `Refused { verdict }` 直接按 `Verdict::gap` 映射成缺口；其它应答原样交给调用方按请求种类核对。
///
/// # 错误
/// socket 文件不存在返回 `TrustedLauncherNotProvisioned`；连接、超时或对端关闭返回
/// `ManagerUnavailable`；帧超限返回 `FrameTooLarge`；载荷形状不合法返回 `ProtocolViolation`。
pub(crate) fn call(socket: &Path, request: &Request) -> Result<Response, ContainmentGap> {
    if !crate::owned_run::absolute_normal(socket)
        || socket.file_name() != Path::new(protocol::LAUNCHER_SOCKET).file_name()
    {
        return Err(ContainmentGap::ProtocolViolation);
    }
    let mut stream = UnixStream::connect(socket).map_err(connect_gap)?;
    stream
        .set_read_timeout(Some(FRAME_TIMEOUT))
        .map_err(|_| ContainmentGap::ManagerUnavailable)?;
    stream
        .set_write_timeout(Some(FRAME_TIMEOUT))
        .map_err(|_| ContainmentGap::ManagerUnavailable)?;
    let payload = serde_json::to_vec(request).map_err(|_| ContainmentGap::ProtocolViolation)?;
    let frame = ipc::encode_frame(&payload)?;
    stream
        .write_all(&frame)
        .map_err(|_| ContainmentGap::ManagerUnavailable)?;
    let reply = read_reply(&mut stream)?;
    let payload = ipc::decode_frame(&reply)?;
    ipc::payload_acceptable(payload)?;
    let response: Response =
        serde_json::from_slice(payload).map_err(|_| ContainmentGap::ProtocolViolation)?;
    match response {
        Response::Refused { verdict } => Err(verdict.gap()),
        accepted => Ok(accepted),
    }
}

/// 读一帧应答：先 4 字节大端长度，再按长度读满载荷；长度越界时不分配、不读取。
fn read_reply(stream: &mut UnixStream) -> Result<Vec<u8>, ContainmentGap> {
    let mut header = [0u8; 4];
    stream
        .read_exact(&mut header)
        .map_err(|_| ContainmentGap::ManagerUnavailable)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > FRAME_MAX_BYTES {
        return Err(ContainmentGap::FrameTooLarge);
    }
    let mut reply = vec![0u8; 4 + length];
    reply[..4].copy_from_slice(&header);
    stream
        .read_exact(&mut reply[4..])
        .map_err(|_| ContainmentGap::ManagerUnavailable)?;
    Ok(reply)
}

/// 连接失败的分类：socket 文件根本不存在＝管理员尚未置备 launcher；其余都是 manager 不可用。
fn connect_gap(error: io::Error) -> ContainmentGap {
    if error.kind() == io::ErrorKind::NotFound {
        ContainmentGap::TrustedLauncherNotProvisioned
    } else {
        ContainmentGap::ManagerUnavailable
    }
}
