//! `studiolo-sync-launcher serve`：私有 Unix socket 循环。每个连接一帧请求、一帧应答；对端凭据
//! 用 SO_PEERCRED 实测（不看自报字段），只接受 owner uid／gid。socket 目录与 sessions 目录只在这里建。

use crate::config::{self, ConfigFault, LauncherConfig};
use crate::handler::Handler;
use crate::protocol::{
    self, FRAME_MAX_BYTES, PeerCredentials, Request, Response, Verdict, decode_request,
    encode_response, peer_acceptable,
};
use crate::unit::Manager;
use rustix::fs::Mode;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

/// 单帧读写超时。
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// socket 目录、socket 与 sessions 目录的位置；生产固定，集成测试指向临时目录。
#[derive(Clone, Debug)]
pub struct Layout {
    /// 0750 root:owner 组。
    pub socket_dir: PathBuf,
    /// 0660 的 socket 文件。
    pub socket: PathBuf,
    /// 0700 root 的 sessions 父目录。
    pub sessions: PathBuf,
}

impl Layout {
    /// 生产固定位置：`config::SOCKET_DIR`、`protocol::LAUNCHER_SOCKET`、`config::SESSIONS_DIR`。
    #[must_use]
    pub fn production() -> Self {
        Self {
            socket_dir: PathBuf::from(config::SOCKET_DIR),
            socket: PathBuf::from(protocol::LAUNCHER_SOCKET),
            sessions: PathBuf::from(config::SESSIONS_DIR),
        }
    }
}

/// `serve` 子命令：读固定配置，必须是 root，然后在固定位置服务直到进程被停止。
pub fn run() -> ExitCode {
    let config = match LauncherConfig::load() {
        Ok(config) => config,
        Err(ConfigFault::Unsafe) => return fail("固定配置不安全：须为 0600 root 单链接普通文件"),
        Err(ConfigFault::Malformed) => return fail("固定配置形状不合法"),
    };
    if rustix::process::geteuid().as_raw() != 0 {
        return fail("serve 必须以 root 运行");
    }
    match serve_with(config, Manager::system(), &Layout::production()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(&format!("socket 服务失败：{error}")),
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(1)
}

/// 建目录与 socket 后循环接受连接；集成测试用临时 `Layout` 与 `--user` manager 调用同一条路径。
///
/// # Errors
/// 建目录、改权限、绑定或接受连接失败返回 IO 错误。
pub fn serve_with(config: LauncherConfig, manager: Manager, layout: &Layout) -> io::Result<()> {
    rustix::process::umask(Mode::from_raw_mode(0o077));
    prepare(layout, &config)?;
    let listener = UnixListener::bind(&layout.socket)?;
    own(&layout.socket, config.owner_gid)?;
    mode(&layout.socket, Mode::from_raw_mode(0o660))?;
    let mut handler = Handler::new(config, manager, layout.sessions.clone());
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => serve_one(stream, &mut handler),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn prepare(layout: &Layout, config: &LauncherConfig) -> io::Result<()> {
    ensure_dir(&layout.socket_dir)?;
    own(&layout.socket_dir, config.owner_gid)?;
    mode(&layout.socket_dir, Mode::from_raw_mode(0o750))?;
    ensure_dir(&layout.sessions)?;
    mode(&layout.sessions, Mode::from_raw_mode(0o700))?;
    match std::fs::remove_file(&layout.socket) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn ensure_dir(path: &Path) -> io::Result<()> {
    match std::fs::create_dir(path) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        result => result,
    }
}

fn own(path: &Path, gid: u32) -> io::Result<()> {
    rustix::fs::chown(path, None, Some(rustix::process::Gid::from_raw(gid)))
        .map_err(io::Error::from)
}

fn mode(path: &Path, mode: Mode) -> io::Result<()> {
    rustix::fs::chmod(path, mode).map_err(io::Error::from)
}

fn serve_one(mut stream: UnixStream, handler: &mut Handler) {
    let _ = stream.set_read_timeout(Some(FRAME_TIMEOUT));
    let _ = stream.set_write_timeout(Some(FRAME_TIMEOUT));
    let response = match accept(&mut stream, handler) {
        Ok(request) => handler.handle(request),
        Err(verdict) => Response::Refused { verdict },
    };
    let frame = encode_response(&response).or_else(|_| {
        encode_response(&Response::Refused {
            verdict: Verdict::Budget,
        })
    });
    if let Ok(frame) = frame {
        let _ = stream.write_all(&frame);
    }
}

fn accept(stream: &mut UnixStream, handler: &Handler) -> Result<Request, Verdict> {
    let peer = peer(stream)?;
    let config = handler.config();
    peer_acceptable(&peer, config.owner_uid, config.owner_gid).map_err(|_| Verdict::Peer)?;
    let frame = read_frame(stream)?;
    decode_request(&frame)
}

#[cfg(target_os = "linux")]
fn peer(stream: &UnixStream) -> Result<PeerCredentials, Verdict> {
    let credentials = rustix::net::sockopt::socket_peercred(stream).map_err(|_| Verdict::Peer)?;
    Ok(PeerCredentials {
        uid: credentials.uid.as_raw(),
        gid: credentials.gid.as_raw(),
        pid: credentials.pid.as_raw_nonzero().get(),
    })
}

#[cfg(not(target_os = "linux"))]
fn peer(_stream: &UnixStream) -> Result<PeerCredentials, Verdict> {
    Err(Verdict::Peer)
}

/// 读一帧：4 字节大端长度加载荷，长度为零或超上限即协议错误；返回完整帧（含长度前缀）。
fn read_frame(stream: &mut UnixStream) -> Result<Vec<u8>, Verdict> {
    let mut header = [0u8; 4];
    stream
        .read_exact(&mut header)
        .map_err(|_| Verdict::Protocol)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > FRAME_MAX_BYTES {
        return Err(Verdict::Protocol);
    }
    let mut frame = vec![0u8; 4 + length];
    frame[..4].copy_from_slice(&header);
    stream
        .read_exact(&mut frame[4..])
        .map_err(|_| Verdict::Protocol)?;
    Ok(frame)
}
