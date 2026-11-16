//! 生产者一侧的部署事实：谁是 worker、socket 和封存放哪、多久销毁、清单里登记哪些包。
//!
//! 只有账号编号、路径、数字和摘要；没有地址、口令。本 Module 不打开这些路径、不创建目录，
//! 也不判断目录真实的属主和权限：那些由运行侧在绑定 socket 和建暂存时逐项核对。

use crate::{ConfigError, absolute_normal};
use serde::{Deserialize, Deserializer};
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 16;
/// 封存正文最短、最长保留多久；短了 worker 来不及传，长了正文副本在盘上留太久。
const RETENTION_MILLIS: std::ops::RangeInclusive<u64> = 600_000..=86_400_000;
/// 一次冻结窗口的预算；上限和冻结预算本身的上限一致。
const FREEZE_MILLIS: std::ops::RangeInclusive<u32> = 1_000..=120_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageDocument {
    name: String,
    version: String,
    digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitDocument {
    unit: String,
    revision: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    worker_uid: u32,
    reader_gid: u32,
    socket: PathBuf,
    staging_parent: PathBuf,
    ephemeral_root: PathBuf,
    source_retention_millis: u64,
    freeze_budget_millis: u32,
    application_packages: Vec<PackageDocument>,
    restic_digest: String,
    deployment: Vec<UnitDocument>,
}

/// 清单里登记的一个固定包。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackagePin {
    name: String,
    version: String,
    digest: String,
}

impl PackagePin {
    /// 包名。
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 固定版本号。
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// 内容摘要（64 位小写十六进制）。
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// 部署清单的一项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentPin {
    unit: String,
    revision: String,
}

impl DeploymentPin {
    /// 部署单元名。
    #[must_use]
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// 该单元的固定版本。
    #[must_use]
    pub fn revision(&self) -> &str {
        &self.revision
    }
}

/// 已校验的生产者部署事实。没有这一段，主服务不监听备份 socket，也不做封存。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupProducerSettings {
    worker_uid: u32,
    reader_gid: u32,
    socket: PathBuf,
    staging_parent: PathBuf,
    ephemeral_root: PathBuf,
    source_retention_millis: u64,
    freeze_budget_millis: u32,
    application_packages: Vec<PackagePin>,
    restic_digest: String,
    deployment: Vec<DeploymentPin>,
}

impl BackupProducerSettings {
    /// 独立 worker 的系统账号；只有它能连备份 socket。
    #[must_use]
    pub fn worker_uid(&self) -> u32 {
        self.worker_uid
    }

    /// 专用读取组：socket 和最终封存只对它开放读。
    #[must_use]
    pub fn reader_gid(&self) -> u32 {
        self.reader_gid
    }

    /// 备份 socket 的固定路径。
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// 封存的固定父目录。
    #[must_use]
    pub fn staging_parent(&self) -> &Path {
        &self.staging_parent
    }

    /// 不进备份的临时数据根；只用来核对它和封存、来源互不重叠。
    #[must_use]
    pub fn ephemeral_root(&self) -> &Path {
        &self.ephemeral_root
    }

    /// 封存正文从内容截止起保留多久。
    #[must_use]
    pub fn source_retention_millis(&self) -> u64 {
        self.source_retention_millis
    }

    /// 一次冻结窗口的预算毫秒数。
    #[must_use]
    pub fn freeze_budget_millis(&self) -> u32 {
        self.freeze_budget_millis
    }

    /// 清单里登记的应用包。
    #[must_use]
    pub fn application_packages(&self) -> &[PackagePin] {
        &self.application_packages
    }

    /// 固定 restic 可执行文件的摘要；和 worker 配置里的是同一个值。
    #[must_use]
    pub fn restic_digest(&self) -> &str {
        &self.restic_digest
    }

    /// 清单里登记的部署单元。
    #[must_use]
    pub fn deployment(&self) -> &[DeploymentPin] {
        &self.deployment
    }

    fn validate(document: Document) -> Result<Self, ConfigError> {
        if document.version != 1
            || matches!(document.worker_uid, 0 | u32::MAX)
            || matches!(document.reader_gid, 0 | u32::MAX)
            || !RETENTION_MILLIS.contains(&document.source_retention_millis)
            || !FREEZE_MILLIS.contains(&document.freeze_budget_millis)
            || !is_digest(&document.restic_digest)
        {
            return Err(ConfigError);
        }
        let paths = [
            &document.socket,
            &document.staging_parent,
            &document.ephemeral_root,
        ];
        if paths
            .iter()
            .any(|path| !absolute_normal(path) || path.parent().is_none())
            // pathname socket 的长度上限（Linux 108 字节，留出结尾的 NUL 和余量）。
            || document.socket.as_os_str().len() > 100
        {
            return Err(ConfigError);
        }
        for (index, left) in paths.iter().enumerate() {
            for right in &paths[index + 1..] {
                if left.starts_with(right) || right.starts_with(left) {
                    return Err(ConfigError);
                }
            }
        }
        if !listed(&document.application_packages) || !listed(&document.deployment) {
            return Err(ConfigError);
        }
        let mut application_packages = Vec::with_capacity(document.application_packages.len());
        for package in document.application_packages {
            if !is_label(&package.name)
                || !is_label(&package.version)
                || !is_digest(&package.digest)
            {
                return Err(ConfigError);
            }
            application_packages.push(PackagePin {
                name: package.name,
                version: package.version,
                digest: package.digest,
            });
        }
        let mut deployment = Vec::with_capacity(document.deployment.len());
        for unit in document.deployment {
            if !is_label(&unit.unit) || !is_label(&unit.revision) {
                return Err(ConfigError);
            }
            deployment.push(DeploymentPin {
                unit: unit.unit,
                revision: unit.revision,
            });
        }
        Ok(Self {
            worker_uid: document.worker_uid,
            reader_gid: document.reader_gid,
            socket: document.socket,
            staging_parent: document.staging_parent,
            ephemeral_root: document.ephemeral_root,
            source_retention_millis: document.source_retention_millis,
            freeze_budget_millis: document.freeze_budget_millis,
            application_packages,
            restic_digest: document.restic_digest,
            deployment,
        })
    }
}

fn listed<T>(entries: &[T]) -> bool {
    !entries.is_empty() && entries.len() <= MAX_ENTRIES
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 包名、版本、单元名：短的可见 ASCII，不含空白和路径分隔符。
fn is_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'@'))
}

/// 供 `Config` 的文档接线；缺字段即视为生产者一侧未启用。
pub fn decode<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<Option<BackupProducerSettings>, D::Error> {
    struct Object;
    impl<'de> serde::de::Visitor<'de> for Object {
        type Value = Option<BackupProducerSettings>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("备份生产者配置对象")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
            let document =
                Document::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
            BackupProducerSettings::validate(document)
                .map(Some)
                .map_err(serde::de::Error::custom)
        }
    }
    input.deserialize_map(Object)
}

#[cfg(test)]
#[path = "producer_tests.rs"]
mod tests;
