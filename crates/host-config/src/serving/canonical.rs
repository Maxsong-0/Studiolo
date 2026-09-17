//! 在原四根审批之上增加既有 Broker data；所有文件系统规则继续复用同一内核。

use super::{Config, ConfigError, Path, Root, ServingRoots, config_file};

/// canonical 服务的封闭根快照；只用于可信装配，不是 HTTP 授权凭据。
///
/// 五个目录两两分离，配置文件不能位于写者数据内。配置的共同父目录仍可
/// 容纳这些独立目录。此类型不创建目录、修改权限或提供存储初始化能力。
pub struct CanonicalServingRoots {
    base: ServingRoots,
    writer_data: Root,
    content: Option<crate::HumanContentScope>,
    file_operations: Option<crate::FileOperationPolicy>,
}

impl CanonicalServingRoots {
    pub(crate) fn new(
        config: &Config,
        vault: &Path,
        assets: &Path,
        writer_data: &Path,
    ) -> Result<Self, ConfigError> {
        let snapshot = Self {
            base: ServingRoots::new(config, vault, assets)?,
            writer_data: Root::new(writer_data, true)?,
            content: config.document.human_content.clone(),
            file_operations: config.file_operation_policy()?,
        };
        snapshot.revalidate()?;
        Ok(snapshot)
    }

    /// 经批准的 canonical 目录，仅供 Broker 打开及真实监视注册。
    #[must_use]
    pub fn vault_directory(&self) -> &Path {
        &self.base.roots[2].path
    }

    /// 既有私有 Broker 数据目录，不得由调用者自动初始化。
    #[must_use]
    pub fn writer_data_directory(&self) -> &Path {
        &self.writer_data.path
    }

    /// 批准时 canonical 目录的设备/inode。
    #[must_use]
    pub fn vault_identity(&self) -> (u64, u64) {
        self.base.vault_identity()
    }

    /// 批准时写者数据目录的设备/inode。
    #[must_use]
    pub fn writer_data_identity(&self) -> (u64, u64) {
        self.writer_data.identity
    }

    /// 供资源句柄打开后与批准对象比较。
    #[must_use]
    pub fn assets_identity(&self) -> (u64, u64) {
        self.base.assets_identity()
    }

    /// 只由同一私有主人配置授予的正文范围；缺省保持只读。
    #[must_use]
    pub fn human_content(&self) -> Option<&crate::HumanContentScope> {
        self.content.as_ref()
    }

    /// 同一私有文件的显式整文件操作批准；默认关闭。
    #[must_use]
    pub fn file_operation_policy(&self) -> Option<&crate::FileOperationPolicy> {
        self.file_operations.as_ref()
    }

    /// 复核配置字节/身份、五根身份与全部分离关系，不接受替换后的新基准。
    pub fn revalidate(&self) -> Result<(), ConfigError> {
        self.base.revalidate()?;
        self.writer_data.check()?;
        for root in &self.base.roots {
            config_file::disjoint(&self.writer_data.path, &root.path)?;
        }
        config_file::outside(&self.base.file.path, self.writer_data.identity)?;
        self.base.revalidate()?;
        self.writer_data.check()
    }
}
