//! 已连接 UDS 的单次请求/应答；不监听、不选路径，实际 peer UID 必须来自可信部署配置。
use super::AUDIT_MESSAGE_MAX_BYTES;
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    net::{AddressFamily, SocketType, sockopt},
};
use std::{
    io::{ErrorKind, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

/// 套接字失败均不泄漏消息、路径或底层错误。客户端不得把失联当成未落账。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditTransportError {
    /// 实际内核 peer 与批准身份不匹配。
    Peer,
    /// 零长、越界、多帧、截断或重复使用。
    Frame,
    /// 整次交互原截止时间已到。
    Deadline,
    /// 连接 IO 不可用，副作用结果可能未知。
    Unavailable,
}
impl std::fmt::Display for AuditTransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "备份审计通信未完成：{self:?}")
    }
}
impl std::error::Error for AuditTransportError {}
type Result<T> = std::result::Result<T, AuditTransportError>;

/// 一个连接只收一帧、只发一帧，并以半关闭确定边界；任何失败后不重用连接。
pub struct AuditSocket {
    stream: UnixStream,
    peer_uid: u32,
    deadline: Instant,
    read: bool,
    written: bool,
    poisoned: bool,
}
impl AuditSocket {
    /// 接受/连接由启动层负责；只核已连接 Linux AF_UNIX/SOCK_STREAM 的真实 peer。
    pub fn new(stream: UnixStream, expected_peer_uid: u32, deadline: Instant) -> Result<Self> {
        let socket = Self {
            stream,
            peer_uid: expected_peer_uid,
            deadline: deadline.min(Instant::now() + Duration::from_secs(10)),
            read: false,
            written: false,
            poisoned: false,
        };
        socket.check()?;
        socket
            .stream
            .set_nonblocking(true)
            .map_err(|_| AuditTransportError::Unavailable)?;
        Ok(socket)
    }
    /// 发完固定帧后立即关闭写半边；不是刷新成功就推断对端已持久化。
    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        if self.poisoned {
            return Err(AuditTransportError::Frame);
        }
        self.poisoned = true;
        let result = self.send_inner(bytes);
        if result.is_ok() {
            self.poisoned = false;
        }
        result
    }
    fn send_inner(&mut self, bytes: &[u8]) -> Result<()> {
        self.check()?;
        if self.written || bytes.is_empty() || bytes.len() > AUDIT_MESSAGE_MAX_BYTES {
            return Err(AuditTransportError::Frame);
        }
        self.written = true;
        let size = u32::try_from(bytes.len())
            .map_err(|_| AuditTransportError::Frame)?
            .to_be_bytes();
        self.write(&size)?;
        self.write(bytes)?;
        self.stream
            .shutdown(Shutdown::Write)
            .map_err(|_| AuditTransportError::Unavailable)?;
        self.check()
    }
    /// 长度先限额，再分配正文；必须观察到 EOF，尾随任何一个字节都拒绝。
    pub fn receive(&mut self) -> Result<Vec<u8>> {
        if self.poisoned {
            return Err(AuditTransportError::Frame);
        }
        self.poisoned = true;
        let result = self.receive_inner();
        if result.is_ok() {
            self.poisoned = false;
        }
        result
    }
    fn receive_inner(&mut self) -> Result<Vec<u8>> {
        self.check()?;
        if self.read {
            return Err(AuditTransportError::Frame);
        }
        self.read = true;
        let mut header = [0_u8; 4];
        self.fill(&mut header)?;
        let size = u32::from_be_bytes(header) as usize;
        if size == 0 || size > AUDIT_MESSAGE_MAX_BYTES {
            return Err(AuditTransportError::Frame);
        }
        let mut bytes = vec![0; size];
        self.fill(&mut bytes)?;
        let mut extra = [0];
        if self.read_some(&mut extra)? != 0 {
            return Err(AuditTransportError::Frame);
        }
        self.check()?;
        Ok(bytes)
    }
    fn check(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            return Err(AuditTransportError::Deadline);
        }
        if matches!(self.peer_uid, 0 | u32::MAX)
            || sockopt::socket_domain(&self.stream).map_err(|_| AuditTransportError::Peer)?
                != AddressFamily::UNIX
            || sockopt::socket_type(&self.stream).map_err(|_| AuditTransportError::Peer)?
                != SocketType::STREAM
            || sockopt::socket_peercred(&self.stream)
                .map_err(|_| AuditTransportError::Peer)?
                .uid
                .as_raw()
                != self.peer_uid
        {
            return Err(AuditTransportError::Peer);
        }
        Ok(())
    }
    fn write(&mut self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            self.check()?;
            match self.stream.write(bytes) {
                Ok(0) => return Err(AuditTransportError::Unavailable),
                Ok(count) => bytes = &bytes[count..],
                Err(e) if e.kind() == ErrorKind::WouldBlock => self.wait(PollFlags::OUT)?,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => return Err(AuditTransportError::Unavailable),
            }
        }
        self.check()
    }
    fn fill(&mut self, mut bytes: &mut [u8]) -> Result<()> {
        while !bytes.is_empty() {
            let count = self.read_some(bytes)?;
            if count == 0 {
                return Err(AuditTransportError::Frame);
            }
            bytes = &mut bytes[count..];
        }
        Ok(())
    }
    fn read_some(&mut self, bytes: &mut [u8]) -> Result<usize> {
        loop {
            self.check()?;
            match self.stream.read(bytes) {
                Ok(count) => return Ok(count),
                Err(e) if e.kind() == ErrorKind::WouldBlock => self.wait(PollFlags::IN)?,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => return Err(AuditTransportError::Unavailable),
            }
        }
    }
    fn wait(&self, flags: PollFlags) -> Result<()> {
        self.check()?;
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        let timeout = Timespec {
            tv_sec: remaining.as_secs() as i64,
            tv_nsec: i64::from(remaining.subsec_nanos()),
        };
        match poll(&mut [PollFd::new(&self.stream, flags)], Some(&timeout)) {
            Ok(_) | Err(rustix::io::Errno::INTR) => self.check(),
            Err(_) => Err(AuditTransportError::Unavailable),
        }
    }
}
