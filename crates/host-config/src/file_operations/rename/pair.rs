use crate::{ConfigError, config_file};
use studiolo_core::{VaultPath, VaultUuid};

/// 双路径的封闭配置许可；不表示正文合格、源存在、目标缺失或真实身份已授权。
/// ```compile_fail
/// let pair: studiolo_host_config::ApprovedRenamePair = serde_json::from_str("{}").unwrap();
/// ```
pub struct ApprovedRenamePair {
    pub(super) file: config_file::Snapshot,
    pub(super) vault: VaultUuid,
    pub(super) from: VaultPath,
    pub(super) to: VaultPath,
    pub(super) hash: String,
    pub(super) epoch: u64,
    pub(super) limit: u64,
}

impl ApprovedRenamePair {
    /// 最终真实身份核验返回后仍须调用；不将新配置补成旧配对。
    pub fn revalidate(&self) -> Result<(), ConfigError> {
        self.file.revalidate()
    }
    /// 已批准的逻辑 Vault；真实根另由 owner 固定。
    #[must_use]
    pub fn vault_uuid(&self) -> VaultUuid {
        self.vault
    }
    /// 固定源路径，不可在重试中替换。
    #[must_use]
    pub fn from_path(&self) -> &VaultPath {
        &self.from
    }
    /// 固定目标路径；本许可不创建其父目录。
    #[must_use]
    pub fn to_path(&self) -> &VaultPath {
        &self.to
    }
    /// 私有登记的文本类别；不能从这个字符串推断实际正文已经通过核验。
    #[must_use]
    pub fn class(&self) -> &'static str {
        "markdown_text_v1"
    }
    /// 该类别要求的文本表示，不是客户端提供的 MIME。
    #[must_use]
    pub fn mime(&self) -> &'static str {
        "text/markdown; charset=utf-8"
    }
    /// 精确末尾约束；前段点号仅为名称，不能因此取得类别许可。
    #[must_use]
    pub fn extension(&self) -> &'static str {
        ".md"
    }
    /// 两条规则、父文件操作预算与 4 MiB 文本上限的最小值。
    #[must_use]
    pub fn max_bytes(&self) -> u64 {
        self.limit
    }
    /// 原完整私有配置的 FileOperation 域摘要，与父策略完全一致。
    #[must_use]
    pub fn scope_hash(&self) -> &str {
        &self.hash
    }
    /// 父策略的明确范围代次；不替代持久 owner 的防回退检查。
    #[must_use]
    pub fn scope_epoch(&self) -> u64 {
        self.epoch
    }
}
