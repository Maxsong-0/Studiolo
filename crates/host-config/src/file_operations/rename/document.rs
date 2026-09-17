//! 整页规则必须是严格对象；配置总量已由 Snapshot 以实际字节限制为 16 KiB。
mod rule;
use crate::{ConfigError, HumanContentScope, sync_scope};
pub(super) use rule::Rule;
use serde::{Deserialize, Deserializer};
use studiolo_core::{VaultPath, VaultUuid};

pub(super) const TEXT_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::file_operations) struct Document {
    version: u8,
    whole_folders: Vec<Rule>,
    whole_files: Vec<Rule>,
}

pub(in crate::file_operations) fn decode<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<Option<Document>, D::Error> {
    object(input).map(Some)
}

pub(super) fn object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    input: D,
) -> Result<T, D::Error> {
    struct Object<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Object<T> {
        type Value = T;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("显式重命名批准对象")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<T, M::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    input.deserialize_map(Object(std::marker::PhantomData))
}

impl Document {
    pub(in crate::file_operations) fn validate(
        &self,
        human: &HumanContentScope,
        vault: VaultUuid,
        generated: &[VaultPath],
    ) -> Result<(), ConfigError> {
        if self.version != 1
            || self.whole_folders.len() > 64
            || self.whole_files.len() > 128
            || (self.whole_folders.is_empty() && self.whole_files.is_empty())
        {
            return Err(ConfigError);
        }
        let (protected, human_generated) = human.sync_exclusions(vault)?;
        let exclusions: Vec<_> = protected
            .iter()
            .chain(&human_generated)
            .chain(generated)
            .collect();
        let paths: Vec<_> = self.paths().collect();
        sync_scope::validate_rename_paths(
            paths
                .iter()
                .copied()
                .chain(human.rename_registered_paths())
                .chain(generated),
        )?;
        for (index, rule) in self
            .whole_folders
            .iter()
            .chain(&self.whole_files)
            .enumerate()
        {
            if rule.class != "markdown_text_v1"
                || rule.max_bytes == 0
                || rule.max_bytes > TEXT_LIMIT
                || !human.allows_whole_file(vault, &rule.path)
                || exclusions.iter().any(|path| {
                    sync_scope::rename_within(&rule.path, path)
                        || sync_scope::rename_within(path, &rule.path)
                })
                || paths[..index].iter().any(|path| {
                    sync_scope::rename_within(&rule.path, path)
                        || sync_scope::rename_within(path, &rule.path)
                })
            {
                return Err(ConfigError);
            }
        }
        if self
            .whole_files
            .iter()
            .any(|rule| !rule.path.as_str().ends_with(".md"))
        {
            return Err(ConfigError);
        }
        Ok(())
    }

    pub(super) fn paths(&self) -> impl Iterator<Item = &VaultPath> {
        self.whole_folders
            .iter()
            .chain(&self.whole_files)
            .map(|rule| &rule.path)
    }

    pub(super) fn rule(&self, path: &VaultPath) -> Option<&Rule> {
        self.whole_files
            .iter()
            .find(|rule| rule.path == *path)
            .or_else(|| {
                self.whole_folders
                    .iter()
                    .find(|rule| rule.path != *path && sync_scope::rename_within(path, &rule.path))
            })
    }
}
