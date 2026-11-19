//! 备份配置的独立私有文件：主人的备份审批（`backup`）和生产者部署事实（`backup_producer`）成对放在这里。
//!
//! 为什么不放进主人配置：双向 Bridge 的持久绑定 `scope_hash` 是整份主人配置字节的摘要
//! （`sync_scope.rs`），往里加任何一段都会让 Bridge 绑定对不上、人侧起不来。
//! 备份单独一份文件，启用或调整备份不碰 Bridge 的绑定（T-216，2026-11-18 用户拍板）。
//!
//! 读法和主人配置一样：0600、本人所有、父目录 0700、最多 16 KiB、读取期间对象不变；未知字段失败关闭。

use super::{BackupApproval, BackupProducerSettings};
use crate::{Config, ConfigError, config_file};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    // 两段都没有 `default`：缺哪一段都在解码时拒绝，不留「批了但不知道往哪放」的半开状态。
    #[serde(deserialize_with = "super::decode")]
    backup: Option<BackupApproval>,
    #[serde(deserialize_with = "super::producer::decode")]
    backup_producer: Option<BackupProducerSettings>,
}

/// 已校验的备份配置；只能经 [`BackupConfig::load`] 从私有文件取得。
///
/// ```compile_fail
/// let config: studiolo_host_config::backup::BackupConfig = serde_json::from_str("{}").unwrap();
/// ```
pub struct BackupConfig {
    approval: BackupApproval,
    producer: BackupProducerSettings,
}

impl BackupConfig {
    /// 读备份配置文件，并按同一份主人配置核对：备份的 socket、封存父目录、临时根
    /// 都不能和 Auth／Audit 私有目录互相嵌套。
    ///
    /// 不打开 socket、不建目录，也不代表目标真的可达。
    pub fn load(path: &Path, owner: &Config) -> Result<Self, ConfigError> {
        let file = config_file::Snapshot::read(path)?;
        if file.bytes().iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
            return Err(ConfigError);
        }
        let document: Document = serde_json::from_slice(file.bytes()).map_err(|_| ConfigError)?;
        let (Some(approval), Some(producer)) = (document.backup, document.backup_producer) else {
            return Err(ConfigError);
        };
        if document.version != 1 {
            return Err(ConfigError);
        }
        for path in [
            producer.socket(),
            producer.staging_parent(),
            producer.ephemeral_root(),
        ] {
            for private in [owner.auth_directory(), owner.audit_directory()] {
                if path.starts_with(private) || private.starts_with(path) {
                    return Err(ConfigError);
                }
            }
        }
        Ok(Self { approval, producer })
    }

    /// 主人的备份审批。
    #[must_use]
    pub fn approval(&self) -> &BackupApproval {
        &self.approval
    }

    /// 生产者的部署事实。
    #[must_use]
    pub fn producer(&self) -> &BackupProducerSettings {
        &self.producer
    }
}
