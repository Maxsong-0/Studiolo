//! 整文件同步范围来自私有配置，既不授予根能力，也不改变浏览器的人写范围。

mod document;
mod paths;
mod traversal;
mod types;
mod union;

pub(crate) use document::{Document, decode};
pub use traversal::{SyncDirectoryDecision, SyncObservationSelection, SyncTraversalRule};
pub use types::*;

use crate::{Config, ConfigError, config_file};
use sha2::{Digest, Sha256};
use studiolo_core::VaultPath;

/// 封闭整文件批准；只能从同一私有主人配置取得，没有公开构造或反序列化入口。
///
/// ```compile_fail
/// let scope: studiolo_host_config::SyncScope = serde_json::from_str("{}").unwrap();
/// ```
pub struct SyncScope {
    file: config_file::Snapshot,
    binding: SyncBinding,
    document: Document,
    protected: Vec<VaultPath>,
    generated: Vec<VaultPath>,
}

impl SyncScope {
    pub(crate) fn new(config: &Config, document: &Document) -> Result<Self, ConfigError> {
        config.file.revalidate()?;
        let (protected, generated) = document.validate(config.document.human_content.as_ref())?;
        let mut hash = Sha256::new();
        hash.update(b"studiolo.sync-scope.private-config.v1\0");
        hash.update((config.file.bytes().len() as u64).to_be_bytes());
        hash.update(config.file.bytes());
        let scope = Self {
            file: config.file.clone(),
            binding: SyncBinding {
                vault_uuid: document.vault_uuid,
                link_id: document::identity(&document.link_id)?,
                remote_vault: document::identity(&document.remote_vault)?,
                scope_epoch: document.scope_epoch,
                scope_hash: hash.finalize().into(),
            },
            document: document.clone(),
            protected,
            generated,
        };
        scope.revalidate()?;
        Ok(scope)
    }

    /// 只含逻辑身份与实际完整配置摘要；真实根身份仍须由可信装配另行绑定。
    #[must_use]
    pub fn binding(&self) -> &SyncBinding {
        &self.binding
    }

    /// 每次观察、派发及最终安装前均须重验；不能证明与后续 IO 的系统级原子性。
    pub fn revalidate(&self) -> Result<(), ConfigError> {
        self.file.revalidate()
    }

    /// 全部目录规则与精确文件规则里最大的单对象字节上限。
    ///
    /// Bridge 执行器的保守预算据此设定（`Limits::conservative`）；它是内部策略上限，
    /// 不是任何 Sync 服务配额，也不放宽逐规则的 `max_bytes`。
    #[must_use]
    pub fn max_bytes(&self) -> u64 {
        self.document
            .whole_folders
            .iter()
            .map(|rule| rule.policy.max_bytes)
            .chain(
                self.document
                    .whole_files
                    .iter()
                    .map(|rule| rule.policy.max_bytes),
            )
            .max()
            .unwrap_or(0)
    }

    /// 读取正文前取得该路径的类型/字节约束；None 表示不得读取该路径正文。
    ///
    /// 只核对单路径与可信配置，不证明 Missing、扫描完整、实际类型或摘要。
    /// 不授予根或写者能力，不能代替最终的完整 approve_union。
    pub fn policy_for_observation(
        &self,
        binding: &SyncBinding,
        path: &VaultPath,
    ) -> Result<Option<SyncFilePolicy>, SyncScopeError> {
        self.revalidate()
            .map_err(|_| SyncScopeError::ConfigurationChanged)?;
        let result = self.observation_policy(binding, path);
        self.revalidate()
            .map_err(|_| SyncScopeError::ConfigurationChanged)?;
        result
    }

    fn observation_policy(
        &self,
        binding: &SyncBinding,
        path: &VaultPath,
    ) -> Result<Option<SyncFilePolicy>, SyncScopeError> {
        if &self.binding != binding {
            return Err(SyncScopeError::BindingMismatch);
        }
        let mut index = paths::Index::default();
        for registered in self
            .document
            .whole_folders
            .iter()
            .map(|rule| &rule.prefix)
            .chain(self.document.whole_files.iter().map(|rule| &rule.path))
            .chain(&self.protected)
            .chain(&self.generated)
        {
            index.add(registered)?;
        }
        // 先排除畸形/别名，不能先用 None 吞掉针对登记路径的碰撞。
        index.add(path)?;
        match self.policy(path) {
            Ok(policy) => Ok(Some(policy)),
            Err(SyncScopeError::ScopeChangeRequired) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// 原子批准完整基线/两侧观察并集；任一排除、缺项或超限均无部分结果。
    ///
    /// 中间判定只读元数据；前后重验实际配置。观察完整性、正文类型/摘要和稳定根
    /// 必须由各 IO owner 证明，不能把自报类型或此返回值当作 Broker 授权。
    pub fn approve_union(
        &self,
        binding: &SyncBinding,
        baseline: &[SyncPathEvidence],
        canonical: &[SyncPathEvidence],
        mirror: &[SyncPathEvidence],
    ) -> Result<ApprovedSyncUnion, SyncScopeError> {
        self.revalidate()
            .map_err(|_| SyncScopeError::ConfigurationChanged)?;
        let approved = union::approve(self, binding, baseline, canonical, mirror)?;
        self.revalidate()
            .map_err(|_| SyncScopeError::ConfigurationChanged)?;
        Ok(approved)
    }

    fn policy(&self, path: &VaultPath) -> Result<SyncFilePolicy, SyncScopeError> {
        if paths::hard_excluded(path)
            || self
                .protected
                .iter()
                .any(|prefix| paths::within(path, prefix))
            || self.generated.iter().any(|item| paths::within(path, item))
        {
            return Err(SyncScopeError::ScopeChangeRequired);
        }
        self.document
            .whole_files
            .iter()
            .find(|rule| rule.path == *path)
            .map(|rule| rule.policy.clone())
            .or_else(|| {
                self.document
                    .whole_folders
                    .iter()
                    .find(|rule| paths::within(path, &rule.prefix) && *path != rule.prefix)
                    .map(|rule| rule.policy.clone())
            })
            .ok_or(SyncScopeError::ScopeChangeRequired)
    }
}

// 同一整文件排除表供显式人侧文件操作使用，避免raw/生成保留名跨入口漂移。
pub(crate) fn hard_excluded_whole_file(path: &VaultPath) -> bool {
    paths::hard_excluded(path)
}

// Rename 只复用同一保守名字索引，不能从此取得 Sync 或根能力。
pub(crate) fn validate_rename_paths<'a>(
    paths: impl IntoIterator<Item = &'a VaultPath>,
) -> Result<(), ConfigError> {
    let mut index = paths::Index::default();
    for path in paths {
        index.add_bounded(path, 4096).map_err(|_| ConfigError)?;
    }
    Ok(())
}

pub(crate) fn rename_within(path: &VaultPath, prefix: &VaultPath) -> bool {
    paths::within(path, prefix)
}
