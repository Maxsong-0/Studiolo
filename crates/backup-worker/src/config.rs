//! 配置只有固定 root 文件这一条来源，不能由 HTTP 请求或环境覆盖构造权限。
mod input;
use crate::{
    ProcessError, Result,
    digest::{hex, is_sha256_hex},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Component, Path, PathBuf},
    time::Instant,
};
use studiolo_backup::{checkpoint::staging::ReaderIdentity, worker::restic::RepositoryRef};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: u32,
    worker_uid: u32,
    producer_uid: u32,
    reader_gid: u32,
    sealed_parent: PathBuf,
    producer_socket: PathBuf,
    repository: String,
    restic_sha256: String,
    /// 仓库用自签证书时，批准的 CA 文件摘要；不给就用系统信任库。
    #[serde(default)]
    repository_ca_sha256: Option<String>,
}
/// 经原 root 文件核验的独立 worker 部署配置；不实现反序列化或公开字段构造。
pub struct WorkerConfig {
    worker_uid: u32,
    producer_uid: u32,
    reader_gid: u32,
    identity: ReaderIdentity,
    sealed_parent: PathBuf,
    producer_socket: PathBuf,
    repository: RepositoryRef,
    restic_sha256: String,
    repository_ca_sha256: Option<String>,
    /// 摘要核对过的 CA 文件路径；只有 [`WorkerConfig::load`] 读过文件后才有。
    repository_ca: Option<&'static str>,
    digest: String,
}
impl WorkerConfig {
    /// 仅读取 /etc/studiolo/backup-worker.json；缺失、链接、权限或身份不符即拒绝。
    /// 配置里批准了 CA 摘要时，同样按 root 文件规则读 /etc/studiolo/backup-worker-ca.pem 并核摘要。
    pub fn load(deadline: Instant) -> Result<Self> {
        let mut config = Self::parse(&input::read(deadline)?, rustix::process::geteuid().as_raw())?;
        let groups = rustix::process::getgroups().map_err(|_| ProcessError::Configuration)?;
        if rustix::process::getegid().as_raw() != config.reader_gid
            && !groups.iter().any(|gid| gid.as_raw() == config.reader_gid)
        {
            return Err(ProcessError::Configuration);
        }
        if let Some(expected) = &config.repository_ca_sha256 {
            verify_ca(&input::read_ca(deadline)?, expected)?;
            config.repository_ca = Some(input::CA_PATH);
        }
        Ok(config)
    }
    fn parse(bytes: &[u8], current_uid: u32) -> Result<Self> {
        if bytes.len() > 16 * 1024 {
            return Err(ProcessError::Configuration);
        }
        let document: Document =
            serde_json::from_slice(bytes).map_err(|_| ProcessError::Configuration)?;
        if document.schema != 1
            || matches!(current_uid, 0 | u32::MAX)
            || document.worker_uid != current_uid
            || document.worker_uid == document.producer_uid
        {
            return Err(ProcessError::Configuration);
        }
        let identity = ReaderIdentity::from_config(document.producer_uid, document.reader_gid)
            .map_err(|_| ProcessError::Configuration)?;
        approved_path(&document.sealed_parent, 4096)?;
        approved_path(&document.producer_socket, 100)?;
        for protected in [
            Path::new("/var/lib/studiolo-backup-worker"),
            Path::new("/run/studiolo-backup-worker"),
            Path::new("/run/credentials"),
        ] {
            if document.sealed_parent.starts_with(protected)
                || protected.starts_with(&document.sealed_parent)
                || document.producer_socket.starts_with(protected)
                || protected.starts_with(&document.producer_socket)
            {
                return Err(ProcessError::Configuration);
            }
        }
        if document
            .producer_socket
            .starts_with(&document.sealed_parent)
            || document
                .sealed_parent
                .starts_with(&document.producer_socket)
        {
            return Err(ProcessError::Configuration);
        }
        if !is_sha256_hex(&document.restic_sha256)
            || document
                .repository_ca_sha256
                .as_deref()
                .is_some_and(|digest| !is_sha256_hex(digest))
        {
            return Err(ProcessError::Configuration);
        }
        let repository =
            RepositoryRef::parse(document.repository).map_err(|_| ProcessError::Configuration)?;
        crate::runner::validate_repository(&repository)?;
        Ok(Self {
            worker_uid: document.worker_uid,
            producer_uid: document.producer_uid,
            reader_gid: document.reader_gid,
            identity,
            sealed_parent: document.sealed_parent,
            producer_socket: document.producer_socket,
            repository,
            restic_sha256: document.restic_sha256,
            repository_ca_sha256: document.repository_ca_sha256,
            repository_ca: None,
            digest: hex(&Sha256::digest(bytes)),
        })
    }
    /// 本 worker 的非 root UID，已与当前实际进程核对。
    #[must_use]
    pub fn worker_uid(&self) -> u32 {
        self.worker_uid
    }
    /// 批准的 App 生产者 UID；UDS peer 验证须使用它。
    #[must_use]
    pub fn producer_uid(&self) -> u32 {
        self.producer_uid
    }
    /// 实际进程必须已经加入的专用读取组。
    #[must_use]
    pub fn reader_gid(&self) -> u32 {
        self.reader_gid
    }
    /// 只读封存的生产者和专用读取组。
    #[must_use]
    pub fn reader_identity(&self) -> ReaderIdentity {
        self.identity
    }
    /// 只允许按外部 seal 引用选择此父根中的固定子目录。
    #[must_use]
    pub fn sealed_parent(&self) -> &Path {
        &self.sealed_parent
    }
    /// 审计代理的固定本地 socket；仍须核实际 peer UID。
    #[must_use]
    pub fn producer_socket(&self) -> &Path {
        &self.producer_socket
    }
    /// 唯一批准的 HTTPS REST 仓库。
    #[must_use]
    pub fn repository(&self) -> &RepositoryRef {
        &self.repository
    }
    /// 原可执行文件期望摘要。
    #[must_use]
    pub fn restic_sha256(&self) -> &str {
        &self.restic_sha256
    }
    /// 原配置 JSON 的字节摘要，供部署证据引用。
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
    /// 批准的仓库 CA 文件摘要；不用自签证书的仓库为 `None`。
    #[must_use]
    pub fn repository_ca_sha256(&self) -> Option<&str> {
        self.repository_ca_sha256.as_deref()
    }
    /// 摘要已核对的 CA 文件路径，交给 restic 的 `--cacert`；配置没批准 CA 时为 `None`。
    #[must_use]
    pub fn repository_ca(&self) -> Option<&'static str> {
        self.repository_ca
    }
}
/// CA 文件必须是批准的那一份（字节摘要相同），且看起来是 PEM 证书；不是就拒绝启动，不退回系统信任库。
fn verify_ca(bytes: &[u8], expected: &str) -> Result<()> {
    let text = std::str::from_utf8(bytes).map_err(|_| ProcessError::Configuration)?;
    if hex(&Sha256::digest(bytes)) != expected
        || !text.contains("-----BEGIN CERTIFICATE-----")
        || !text.contains("-----END CERTIFICATE-----")
        || text.contains("PRIVATE KEY")
    {
        return Err(ProcessError::Configuration);
    }
    Ok(())
}
fn approved_path(path: &Path, limit: usize) -> Result<()> {
    let text = path.to_str().ok_or(ProcessError::Configuration)?;
    if !path.is_absolute()
        || text.len() > limit
        || text == "/"
        || text.ends_with('/')
        || text.contains("//")
        || text.split('/').any(|c| matches!(c, "." | ".."))
        || !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
        || path
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
    {
        return Err(ProcessError::Configuration);
    }
    Ok(())
}
#[cfg(test)]
#[path = "../tests/support/config.rs"]
mod tests;
