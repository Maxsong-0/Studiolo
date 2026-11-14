//! 逐级原 FD 固定；只能连接 root 配置中的生产者 socket，连接前后不允许路径换根。
mod proof;
use super::super::WorkerConfig;
use proof::Proof;
use rustix::net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
use std::{os::unix::net::UnixStream, path::PathBuf, time::Instant};
use studiolo_audit_ledger::AuditError;
use studiolo_backup::worker::audit::transport::AuditSocket;

pub(super) struct Endpoint {
    path: PathBuf,
    producer: u32,
    group: u32,
    proof: Proof,
}
impl Endpoint {
    pub fn bind(config: &WorkerConfig, deadline: Instant) -> Result<Self, AuditError> {
        let path = config.producer_socket().to_owned();
        let producer = config.producer_uid();
        let group = config.reader_gid();
        let proof = Proof::open(&path, producer, group, deadline)?;
        Ok(Self {
            path,
            producer,
            group,
            proof,
        })
    }
    pub fn check(&self, deadline: Instant) -> Result<(), AuditError> {
        self.proof.same(&Proof::open(
            &self.path,
            self.producer,
            self.group,
            deadline,
        )?)
    }
    pub fn connect(&self, deadline: Instant) -> Result<AuditSocket, AuditError> {
        self.check(deadline)?;
        let socket = connect(&self.path, self.producer, deadline)?;
        self.check(deadline)?;
        Ok(socket)
    }
}
fn connect(
    path: &std::path::Path,
    producer: u32,
    deadline: Instant,
) -> Result<AuditSocket, AuditError> {
    if Instant::now() >= deadline {
        return Err(AuditError::Unavailable);
    }
    let address = SocketAddrUnix::new(path).map_err(|_| AuditError::UnsafeStorage)?;
    let fd = net::socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .map_err(|_| AuditError::Unavailable)?;
    // Unix 队列满时立即失败，不无限等候连接，也不在未知状态自动重连。
    net::connect(&fd, &address).map_err(|_| AuditError::Unavailable)?;
    AuditSocket::new(UnixStream::from(fd), producer, deadline)
        .map_err(|_| AuditError::UnsafeStorage)
}
#[cfg(test)]
#[path = "../../tests/support/audit_endpoint.rs"]
mod tests;
