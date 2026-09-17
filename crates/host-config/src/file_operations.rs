//! 文件删除与最近删除恢复须显式启用；这里只提供范围/预算，不授予身份或根权限。
mod rename;
pub use rename::{ApprovedRenamePair, FileRenamePolicy};

use crate::{Config, ConfigError, HumanContentScope, config_file};
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use studiolo_core::{VaultPath, VaultUuid};

const OBJECT_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    version: u8,
    vault_uuid: VaultUuid,
    scope_epoch: u64,
    effective_object_limit: u64,
    max_tombstones: u32,
    retained_bytes_limit: u64,
    minimum_free_bytes: u64,
    #[serde(default, deserialize_with = "rename::decode")]
    rename: Option<rename::Document>,
}

pub(crate) fn decode<'de, D: Deserializer<'de>>(input: D) -> Result<Option<Document>, D::Error> {
    struct Object;
    impl<'de> serde::de::Visitor<'de> for Object {
        type Value = Option<Document>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("显式文件操作批准对象")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
            Document::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(Some)
        }
    }
    input.deserialize_map(Object)
}

impl Document {
    pub(crate) fn validate(
        &self,
        human: Option<&HumanContentScope>,
        sync: Option<&crate::sync_scope::Document>,
    ) -> Result<(), ConfigError> {
        let human = human.ok_or(ConfigError)?;
        human.sync_exclusions(self.vault_uuid)?;
        if self.version != 1
            || self.scope_epoch == 0
            || self.scope_epoch > i64::MAX as u64
            || self.effective_object_limit == 0
            || self.effective_object_limit > OBJECT_LIMIT
            || !(1..=4096).contains(&self.max_tombstones)
            || self.retained_bytes_limit == 0
            || self.retained_bytes_limit > OBJECT_LIMIT * 2 * 4096
            || self.minimum_free_bytes == 0
        {
            return Err(ConfigError);
        }
        let generated = sync
            .map(|scope| scope.generated_for(self.vault_uuid))
            .transpose()?
            .unwrap_or_default();
        if let Some(rename) = &self.rename {
            rename.validate(human, self.vault_uuid, generated)?;
        }
        Ok(())
    }
}

/// 只能从私有主人配置取得；不接受HTTP给出的开关、范围摘要或预算。
///
/// ```compile_fail
/// let policy: studiolo_host_config::FileOperationPolicy = serde_json::from_str("{}").unwrap();
/// ```
pub struct FileOperationPolicy {
    file: config_file::Snapshot,
    document: Document,
    human: HumanContentScope,
    hash: String,
    rename: Option<FileRenamePolicy>,
    generated: Vec<VaultPath>,
}

impl FileOperationPolicy {
    pub(crate) fn new(config: &Config, document: &Document) -> Result<Self, ConfigError> {
        config.file.revalidate()?;
        document.validate(
            config.document.human_content.as_ref(),
            config.document.sync_scope.as_ref(),
        )?;
        let mut digest = Sha256::new();
        digest.update(b"studiolo.human-file-operations.private-config.v1\0");
        digest.update((config.file.bytes().len() as u64).to_be_bytes());
        digest.update(config.file.bytes());
        let hex = b"0123456789abcdef";
        let hash: String = digest
            .finalize()
            .iter()
            .flat_map(|byte| {
                [
                    char::from(hex[usize::from(byte >> 4)]),
                    char::from(hex[usize::from(byte & 15)]),
                ]
            })
            .collect();
        let generated = sync_generated(config, document.vault_uuid)?;
        let rename = document
            .rename
            .as_ref()
            .map(|rules| FileRenamePolicy::new(config, document, rules, &hash))
            .transpose()?;
        let result = Self {
            file: config.file.clone(),
            document: document.clone(),
            human: config.document.human_content.clone().ok_or(ConfigError)?,
            hash,
            rename,
            generated,
        };
        result.revalidate()?;
        Ok(result)
    }

    /// Rename 必须单独显式批准；取得对象仍须 approve_pair 与最终 revalidate。
    #[must_use]
    pub fn rename_policy(&self) -> Option<&FileRenamePolicy> {
        self.rename.as_ref()
    }

    /// 原配置字节/身份/权限必须仍有效，不能把重载的新配置补成旧批准。
    pub fn revalidate(&self) -> Result<(), ConfigError> {
        self.file.revalidate()
    }

    /// 只批准同一人写范围的普通整文件；任何生成区登记优先，即使哨兵已被删掉。
    /// 此结果不证明文件存在、类型、CAS版本、保全类别或实际登录身份。
    pub fn allows(&self, vault: VaultUuid, path: &VaultPath) -> Result<bool, ConfigError> {
        self.revalidate()?;
        let result = vault == self.document.vault_uuid
            && self.human.allows_whole_file(vault, path)
            && !self
                .generated
                .iter()
                .any(|prefix| crate::sync_scope::rename_within(path, prefix));
        self.revalidate()?;
        Ok(result)
    }

    /// 实际完整私有配置的域分隔摘要，仅是持久请求绑定事实。
    #[must_use]
    pub fn scope_hash(&self) -> &str {
        &self.hash
    }
    /// 范围代次；持久回退由实际Broker/owner核对。
    #[must_use]
    pub fn scope_epoch(&self) -> u64 {
        self.document.scope_epoch
    }
    /// 受批准的逻辑Vault；不指向任意宿主路径。
    #[must_use]
    pub fn vault_uuid(&self) -> VaultUuid {
        self.document.vault_uuid
    }
    /// 单对象工程上限，不冒充Sync账号配额。
    #[must_use]
    pub fn effective_object_limit(&self) -> u64 {
        self.document.effective_object_limit
    }
    /// 墓碑数量上限；没有自动永久删除授权。
    #[must_use]
    pub fn max_tombstones(&self) -> u32 {
        self.document.max_tombstones
    }
    /// L0和隔离对象按最坏逻辑字节计费的总预算。
    #[must_use]
    pub fn retained_bytes_limit(&self) -> u64 {
        self.document.retained_bytes_limit
    }
    /// 每个实际数据设备须保留的空闲空间。
    #[must_use]
    pub fn minimum_free_bytes(&self) -> u64 {
        self.document.minimum_free_bytes
    }
}

fn sync_generated(config: &Config, vault: VaultUuid) -> Result<Vec<VaultPath>, ConfigError> {
    config
        .document
        .sync_scope
        .as_ref()
        .map(|scope| scope.generated_for(vault))
        .transpose()
        .map(|paths| paths.unwrap_or_default().to_vec())
}
