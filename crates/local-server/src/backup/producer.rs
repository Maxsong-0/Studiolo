//! 生产者的固定配置、调度判断和只服务已批准作业的 Unix 监听器。
//! 配置文件缺失时由调用方保持不监听；文件存在但不合法则失败关闭，不改用请求里的路径。
use super::audit::BackupAuditPort;
use rustix::net::sockopt;
use serde::Deserialize;
use std::{
    io::ErrorKind,
    os::unix::net::{UnixListener, UnixStream},
    path::{Component, Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const CONFIG_PATH: &str = "/etc/studiolo/backup-producer.json";

/// 生产者固定配置。字段私有，不能由请求反序列化后直接当权限用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProducerConfig {
    socket: PathBuf,
    worker_uid: u32,
    interval: Duration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: u32,
    socket_path: PathBuf,
    worker_uid: u32,
    interval_seconds: u64,
}

/// 监听或调度失败。不回显路径和套接字正文。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProducerError {
    /// 固定配置缺失、权限不符或字段不合法。
    Configuration,
    /// 套接字无法绑定或接受连接。
    Listen,
    /// 对端不是配置里的 worker。
    Peer,
    /// 本次等待已到截止时间。
    Deadline,
    /// 已批准作业的审计交换没有完成。
    Audit,
}

impl std::fmt::Display for ProducerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("备份生产者未就绪")
    }
}
impl std::error::Error for ProducerError {}

impl ProducerConfig {
    /// 只读固定路径。文件不存在也失败关闭，不创建默认配置。
    pub fn load() -> Result<Self, ProducerError> {
        let metadata =
            std::fs::symlink_metadata(CONFIG_PATH).map_err(|_| ProducerError::Configuration)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > 16 * 1024
        {
            return Err(ProducerError::Configuration);
        }
        let mode = std::os::unix::prelude::MetadataExt::mode(&metadata);
        let uid = std::os::unix::prelude::MetadataExt::uid(&metadata);
        if uid != 0 || mode & 0o022 != 0 {
            return Err(ProducerError::Configuration);
        }
        let bytes = std::fs::read(CONFIG_PATH).map_err(|_| ProducerError::Configuration)?;
        Self::parse(&bytes, rustix::process::geteuid().as_raw())
    }

    /// 校验一份配置字节。`current_uid` 是生产者进程的实际 UID，必须不同于 worker。
    pub fn parse(bytes: &[u8], current_uid: u32) -> Result<Self, ProducerError> {
        let document: Document =
            serde_json::from_slice(bytes).map_err(|_| ProducerError::Configuration)?;
        if document.schema != 1
            || document.worker_uid == 0
            || document.worker_uid == current_uid
            || !(60..=86_400).contains(&document.interval_seconds)
            || !absolute_socket(&document.socket_path)
        {
            return Err(ProducerError::Configuration);
        }
        Ok(Self {
            socket: document.socket_path,
            worker_uid: document.worker_uid,
            interval: Duration::from_secs(document.interval_seconds),
        })
    }

    /// 已批准的独立 worker UID。
    pub fn worker_uid(&self) -> u32 {
        self.worker_uid
    }
    /// 固定套接字路径。
    pub fn socket(&self) -> &Path {
        &self.socket
    }
    /// 两次作业开始之间的最短间隔。
    pub fn interval(&self) -> Duration {
        self.interval
    }
}

/// 下一次备份是否到期。没有上一次完成时间时立即到期，不把间隔当成已经做过。
#[derive(Clone, Debug)]
pub struct BackupSchedule {
    interval: Duration,
    next_due: Option<Instant>,
}
impl BackupSchedule {
    /// 间隔来自固定配置。第一次查询视为到期。
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            next_due: None,
        }
    }
    /// 现在是否应该开始下一次备份。
    pub fn due(&self, now: Instant) -> bool {
        self.next_due.is_none_or(|due| now >= due)
    }
    /// 记录一次开始，并把下一次到期推到间隔之后。
    pub fn mark_started(&mut self, now: Instant) {
        self.next_due = now.checked_add(self.interval);
    }
}

/// 只接受配置中的 worker，并且只服务调用方已经批准的那一个作业。
pub struct ProducerListener {
    listener: UnixListener,
    worker_uid: u32,
}
impl ProducerListener {
    /// 绑定配置中的套接字。路径已被占用则失败，不删除别人的套接字。
    pub fn bind(config: &ProducerConfig) -> Result<Self, ProducerError> {
        let listener = UnixListener::bind(config.socket()).map_err(|_| ProducerError::Listen)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| ProducerError::Listen)?;
        Ok(Self {
            listener,
            worker_uid: config.worker_uid(),
        })
    }

    /// 等到一个连接。对端 UID 不符就关闭，不进入审计交换。
    pub fn serve_one(
        &self,
        port: &BackupAuditPort,
        deadline: Instant,
    ) -> Result<(), ProducerError> {
        let stream = self.accept_worker(deadline)?;
        stream
            .set_nonblocking(false)
            .map_err(|_| ProducerError::Listen)?;
        port.serve_connection(stream, self.worker_uid, deadline)
            .map_err(|_| ProducerError::Audit)
    }

    /// 只接受配置中的 worker。身份不符时不读取请求。
    pub fn accept_worker(&self, deadline: Instant) -> Result<UnixStream, ProducerError> {
        let stream = self.accept(deadline)?;
        let cred = sockopt::socket_peercred(&stream).map_err(|_| ProducerError::Peer)?;
        if cred.uid.as_raw() != self.worker_uid {
            return Err(ProducerError::Peer);
        }
        Ok(stream)
    }

    fn accept(&self, deadline: Instant) -> Result<UnixStream, ProducerError> {
        loop {
            if Instant::now() >= deadline {
                return Err(ProducerError::Deadline);
            }
            match self.listener.accept() {
                Ok((stream, _)) => return Ok(stream),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => return Err(ProducerError::Listen),
            }
        }
    }
}

fn absolute_socket(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        && path.as_os_str().len() <= 100
        && !path.starts_with("/run/credentials")
        && !path.starts_with("/etc/studiolo")
}
