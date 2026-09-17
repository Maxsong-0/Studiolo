//! 配对许可只消费私有登记；实际字节、身份和安装须由同一可信 owner 另行核验。
mod document;
mod pair;
pub(super) use document::{Document, decode};
pub use pair::ApprovedRenamePair;

use crate::{Config, ConfigError, HumanContentScope, config_file, sync_scope};
use studiolo_core::{VaultPath, VaultUuid};

/// 只能由既有私有 file_operations.rename 取得，不授予根或 Broker 能力。
/// ```compile_fail
/// let policy: studiolo_host_config::FileRenamePolicy = serde_json::from_str("{}").unwrap();
/// ```
pub struct FileRenamePolicy {
    file: config_file::Snapshot,
    document: Document,
    human: HumanContentScope,
    generated: Vec<VaultPath>,
    vault: VaultUuid,
    hash: String,
    epoch: u64,
    limit: u64,
}

impl FileRenamePolicy {
    pub(super) fn new(
        config: &Config,
        parent: &super::Document,
        document: &Document,
        hash: &str,
    ) -> Result<Self, ConfigError> {
        config.file.revalidate()?;
        let human = config.document.human_content.clone().ok_or(ConfigError)?;
        let generated = super::sync_generated(config, parent.vault_uuid)?;
        document.validate(&human, parent.vault_uuid, &generated)?;
        let result = Self {
            file: config.file.clone(),
            document: document.clone(),
            human,
            generated,
            vault: parent.vault_uuid,
            hash: hash.to_owned(),
            epoch: parent.scope_epoch,
            limit: parent.effective_object_limit,
        };
        result.revalidate()?;
        Ok(result)
    }

    /// 与父 FileOperationPolicy 同一实际 Snapshot；配置替换或权限变化均失效。
    pub fn revalidate(&self) -> Result<(), ConfigError> {
        self.file.revalidate()
    }

    /// 两路径分别取得同类整文件许可；不读取内容，不证明存在、Missing 或实际 MIME。
    pub fn approve_pair(
        &self,
        vault: VaultUuid,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Result<ApprovedRenamePair, ConfigError> {
        self.revalidate()?;
        let result = self.approve(vault, from, to);
        self.revalidate()?;
        result
    }

    fn approve(
        &self,
        vault: VaultUuid,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Result<ApprovedRenamePair, ConfigError> {
        if [from, to].iter().any(|path| path.as_str().len() > 4096)
            || vault != self.vault
            || [from, to].iter().any(|path| {
                !self.human.allows_whole_file(vault, path)
                    || self
                        .generated
                        .iter()
                        .any(|prefix| sync_scope::rename_within(path, prefix))
            })
        {
            return Err(ConfigError);
        }
        sync_scope::validate_rename_paths(
            self.document
                .paths()
                .chain(self.human.rename_registered_paths())
                .chain(&self.generated)
                .chain([from, to]),
        )?;
        if sync_scope::rename_within(from, to) || sync_scope::rename_within(to, from) {
            return Err(ConfigError);
        }
        let left = self.document.rule(from).ok_or(ConfigError)?;
        let right = self.document.rule(to).ok_or(ConfigError)?;
        if left.class != right.class
            || !from.as_str().ends_with(".md")
            || !to.as_str().ends_with(".md")
        {
            return Err(ConfigError);
        }
        Ok(ApprovedRenamePair {
            file: self.file.clone(),
            vault,
            from: from.clone(),
            to: to.clone(),
            hash: self.hash.clone(),
            epoch: self.epoch,
            limit: self
                .limit
                .min(left.max_bytes)
                .min(right.max_bytes)
                .min(document::TEXT_LIMIT),
        })
    }
}
