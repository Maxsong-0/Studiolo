//! 目录选择与文件策略分开；None 文件策略不能用来决定是否递归目录。
use super::{SyncBinding, SyncFilePolicy, SyncScope, SyncScopeError, paths};
use studiolo_core::VaultPath;

/// 单个目录锚点或精确文件的只读规则；不授予文件系统能力。
pub struct SyncTraversalRule {
    path: VaultPath,
    policy: SyncFilePolicy,
}
impl SyncTraversalRule {
    /// 规范逻辑路径。
    pub fn path(&self) -> &VaultPath {
        &self.path
    }
    /// 显式类型和对象字节约束。
    pub fn policy(&self) -> &SyncFilePolicy {
        &self.policy
    }
}
/// 私有配置导出的封闭选择；无公开字段、构造器或 Deserialize。
pub struct SyncObservationSelection {
    binding: SyncBinding,
    folders: Vec<SyncTraversalRule>,
    files: Vec<SyncTraversalRule>,
    excluded: Vec<VaultPath>,
}
impl SyncObservationSelection {
    /// 与原配置绑定的逻辑身份和实际配置摘要。
    pub fn binding(&self) -> &SyncBinding {
        &self.binding
    }
    /// 必须完整存在的批准目录锚点。
    pub fn folders(&self) -> &[SyncTraversalRule] {
        &self.folders
    }
    /// 精确文件，包括尚未存在的逻辑名称。
    pub fn files(&self) -> &[SyncTraversalRule] {
        &self.files
    }
    /// 除固定硬排除外，额外禁止下探/读正文的前缀。
    pub fn excluded_prefixes(&self) -> &[VaultPath] {
        &self.excluded
    }
}
/// 目录元数据遍历决定，不表示可读取此目录下所有正文。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncDirectoryDecision {
    /// 为完整观察选择范围，必须遍历此目录。
    Traverse,
    /// 不得下探；名字仍计入父目录预算和碰撞检查。
    Skip,
}
impl SyncScope {
    /// 导出只读选择；可信装配负责转交窄数据、最终重验和真正根审批。
    pub fn observation_selection(
        &self,
        binding: &SyncBinding,
    ) -> Result<SyncObservationSelection, SyncScopeError> {
        self.revalidate()
            .map_err(|_| SyncScopeError::ConfigurationChanged)?;
        let result = if &self.binding != binding {
            Err(SyncScopeError::BindingMismatch)
        } else {
            let mut excluded = self.protected.clone();
            excluded.extend(self.generated.iter().cloned());
            excluded.sort();
            excluded.dedup();
            Ok(SyncObservationSelection {
                binding: binding.clone(),
                folders: self
                    .document
                    .whole_folders
                    .iter()
                    .map(|rule| SyncTraversalRule {
                        path: rule.prefix.clone(),
                        policy: rule.policy.clone(),
                    })
                    .collect(),
                files: self
                    .document
                    .whole_files
                    .iter()
                    .map(|rule| SyncTraversalRule {
                        path: rule.path.clone(),
                        policy: rule.policy.clone(),
                    })
                    .collect(),
                excluded,
            })
        };
        self.revalidate()
            .map_err(|_| SyncScopeError::ConfigurationChanged)?;
        result
    }
    /// 目录使用独立判定，先验证全部登记和当前路径拼写，再决定 Traverse/Skip。
    pub fn directory_for_observation(
        &self,
        binding: &SyncBinding,
        path: &VaultPath,
    ) -> Result<SyncDirectoryDecision, SyncScopeError> {
        self.revalidate()
            .map_err(|_| SyncScopeError::ConfigurationChanged)?;
        let result = self.directory_decision(binding, path);
        self.revalidate()
            .map_err(|_| SyncScopeError::ConfigurationChanged)?;
        result
    }
    fn directory_decision(
        &self,
        binding: &SyncBinding,
        path: &VaultPath,
    ) -> Result<SyncDirectoryDecision, SyncScopeError> {
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
        index.add(path)?;
        if paths::hard_excluded(path)
            || self
                .protected
                .iter()
                .chain(&self.generated)
                .any(|prefix| paths::within(path, prefix))
        {
            return Ok(SyncDirectoryDecision::Skip);
        }
        let traverse = self
            .document
            .whole_folders
            .iter()
            .any(|rule| paths::within(path, &rule.prefix) || paths::within(&rule.prefix, path))
            || self
                .document
                .whole_files
                .iter()
                .any(|rule| path != &rule.path && paths::within(&rule.path, path));
        Ok(if traverse {
            SyncDirectoryDecision::Traverse
        } else {
            SyncDirectoryDecision::Skip
        })
    }
}
