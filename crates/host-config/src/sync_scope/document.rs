//! 配置只是整文件批准，不是 headless、Broker 或远端连接参数。

mod rules;
pub(super) use rules::{FileRule, FolderRule};

use super::{SyncAttachmentType, paths};
use crate::{ConfigError, HumanContentScope};
use serde::{Deserialize, Deserializer};
use std::collections::BTreeSet;
use studiolo_core::{VaultPath, VaultUuid};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    pub(super) version: u8,
    pub(super) vault_uuid: VaultUuid,
    pub(super) link_id: String,
    pub(super) remote_vault: String,
    pub(super) scope_epoch: u64,
    pub(super) whole_folders: Vec<FolderRule>,
    pub(super) whole_files: Vec<FileRule>,
    pub(super) generated_documents: Vec<VaultPath>,
}

pub(crate) fn decode<'de, D: Deserializer<'de>>(input: D) -> Result<Option<Document>, D::Error> {
    object(input).map(Some)
}

pub(super) fn object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    input: D,
) -> Result<T, D::Error> {
    struct Object<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Object<T> {
        type Value = T;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("显式整文件同步批准对象")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<T, M::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    input.deserialize_map(Object(std::marker::PhantomData))
}

pub(super) fn identity(text: &str) -> Result<[u8; 16], ConfigError> {
    if text.len() != 32
        || !text
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(ConfigError);
    }
    let mut result = [0; 16];
    for (index, value) in result.iter_mut().enumerate() {
        *value =
            u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).map_err(|_| ConfigError)?;
    }
    if result == [0; 16] {
        return Err(ConfigError);
    }
    Ok(result)
}

impl Document {
    // 只能继承相同逻辑 Vault 的禁止项；不给调用方同步许可。
    pub(crate) fn generated_for(&self, vault: VaultUuid) -> Result<&[VaultPath], ConfigError> {
        if self.vault_uuid != vault {
            return Err(ConfigError);
        }
        Ok(&self.generated_documents)
    }

    pub(crate) fn validate(
        &self,
        human: Option<&HumanContentScope>,
    ) -> Result<(Vec<VaultPath>, Vec<VaultPath>), ConfigError> {
        if self.version != 1
            || self.scope_epoch == 0
            || self.scope_epoch > i64::MAX as u64
            || self.whole_folders.len() > 64
            || self.whole_files.len() > 128
            || self.generated_documents.len() > 128
            || (self.whole_folders.is_empty() && self.whole_files.is_empty())
        {
            return Err(ConfigError);
        }
        identity(&self.link_id)?;
        identity(&self.remote_vault)?;
        let (protected, mut generated) = human
            .map(|scope| scope.sync_exclusions(self.vault_uuid))
            .transpose()?
            .unwrap_or_default();
        let mut unique = BTreeSet::new();
        for path in &self.generated_documents {
            if !unique.insert(path) {
                return Err(ConfigError);
            }
        }
        generated.extend(self.generated_documents.iter().cloned());
        generated.sort();
        generated.dedup();
        let mut index = paths::Index::default();
        let mut approved = Vec::new();
        for rule in &self.whole_folders {
            rule.policy.validate()?;
            approved.push(&rule.prefix);
        }
        for rule in &self.whole_files {
            rule.policy.validate()?;
            approved.push(&rule.path);
        }
        for (i, path) in approved.iter().enumerate() {
            index.add(path).map_err(|_| ConfigError)?;
            if paths::hard_excluded(path)
                || protected.iter().any(|prefix| paths::within(path, prefix))
                || generated.iter().any(|item| paths::within(path, item))
                || approved[..i]
                    .iter()
                    .any(|other| paths::within(path, other) || paths::within(other, path))
            {
                return Err(ConfigError);
            }
        }
        // 禁止项与批准项必须共用同一拼写基准；不能借别名藏起混合文档。
        for path in protected.iter().chain(&generated) {
            index.add(path).map_err(|_| ConfigError)?;
        }
        for file in &self.whole_files {
            if protected
                .iter()
                .chain(&generated)
                .any(|path| paths::within(path, &file.path))
            {
                return Err(ConfigError);
            }
        }
        Ok((protected, generated))
    }
}

impl super::SyncFilePolicy {
    fn validate(&self) -> Result<(), ConfigError> {
        let kinds: BTreeSet<SyncAttachmentType> = self.attachment_types.iter().copied().collect();
        if self.max_bytes == 0
            || self.max_bytes > 256 * 1024 * 1024
            || (!self.markdown && self.attachment_types.is_empty())
            || kinds.len() != self.attachment_types.len()
        {
            return Err(ConfigError);
        }
        Ok(())
    }
}
