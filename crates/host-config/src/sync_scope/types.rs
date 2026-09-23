//! 观察是数据，不是权限；只有封闭的 Scope 能产出核准结果。

use serde::Deserialize;
use std::collections::BTreeMap;
use studiolo_core::{VaultPath, VaultUuid};

/// 显式登记的附件种类；没有由扩展名或通配符自动批准的分支。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub enum SyncAttachmentType {
    /// PNG 图片。
    Png,
    /// JPEG 图片。
    Jpeg,
    /// GIF 图片。
    Gif,
    /// WebP 图片。
    Webp,
    /// PDF 文档。
    Pdf,
    /// MP3 音频。
    Mp3,
    /// MP4 媒体。
    Mp4,
    /// WAV 音频。
    Wav,
    /// ZIP 附件。
    Zip,
    /// 纯文本（T-132 方案第 6 节第 1 条：`_规则/归档/` 的 txt；ADR-025 D12）。
    Txt,
    /// JSON 文本（同上；headless 侧归入 `unsupported` 类）。
    Json,
}

impl TryFrom<String> for SyncAttachmentType {
    type Error = crate::ConfigError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "png" => Ok(Self::Png),
            "jpeg" => Ok(Self::Jpeg),
            "gif" => Ok(Self::Gif),
            "webp" => Ok(Self::Webp),
            "pdf" => Ok(Self::Pdf),
            "mp3" => Ok(Self::Mp3),
            "mp4" => Ok(Self::Mp4),
            "wav" => Ok(Self::Wav),
            "zip" => Ok(Self::Zip),
            "txt" => Ok(Self::Txt),
            "json" => Ok(Self::Json),
            _ => Err(crate::ConfigError),
        }
    }
}

/// 可信读取器根据固定源的实际字节提供的类型；声明类型本身不构成证明。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncFileKind {
    /// 实际 UTF-8 Markdown 正文。
    Markdown,
    /// 经读取器核验的已登记附件类型。
    Attachment(SyncAttachmentType),
}

/// 可公开比较的绑定事实，不是可公开构造的 Sync 能力。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncBinding {
    /// Canonical Vault 的逻辑身份。
    pub vault_uuid: VaultUuid,
    /// 固定同步链身份。
    pub link_id: [u8; 16],
    /// 固定远端 Vault 身份。
    pub remote_vault: [u8; 16],
    /// 显式递增的范围代次；本模块不代替持久 owner 检查回退。
    pub scope_epoch: u64,
    /// 域分隔及完整私有配置实际字节的 SHA-256。
    pub scope_hash: [u8; 32],
}

/// 内容核验后的观察元数据；本模块不打开正文或依靠文件后缀分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncContentMetadata {
    /// 实际内容类别。
    pub kind: SyncFileKind,
    /// 实际字节数。
    pub bytes: u64,
}

/// 显式路径记录；None 是 Missing，不允许以省略条目代替确认缺失。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncPathEvidence {
    /// 规范 Vault 相对路径。
    pub path: VaultPath,
    /// 当前或基线内容；基线两侧同内容的证明仍由 Bridge 核验。
    pub content: Option<SyncContentMetadata>,
}

/// 一条整文件规则的只读约束；不能单独反序列化或构造成批准能力。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncFilePolicy {
    pub(super) markdown: bool,
    pub(super) attachment_types: Vec<SyncAttachmentType>,
    pub(super) max_bytes: u64,
}

impl SyncFilePolicy {
    /// 是否明确允许普通整页 Markdown。
    #[must_use]
    pub fn markdown(&self) -> bool {
        self.markdown
    }

    /// 显式允许的附件类型闭集。
    #[must_use]
    pub fn attachment_types(&self) -> &[SyncAttachmentType] {
        &self.attachment_types
    }

    /// 单个实际对象的字节上限。
    #[must_use]
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
}

/// 全批成功才产出的精确集合；不能公开构造、反序列化或修改。
pub struct ApprovedSyncUnion {
    pub(super) binding: SyncBinding,
    pub(super) paths: BTreeMap<VaultPath, SyncFilePolicy>,
}

impl ApprovedSyncUnion {
    /// 本批绑定；后续实际根 token 必须由可信装配加入。
    #[must_use]
    pub fn binding(&self) -> &SyncBinding {
        &self.binding
    }

    /// 完整、排序且不含碰撞的路径与逐对象约束。
    #[must_use]
    pub fn paths(&self) -> &BTreeMap<VaultPath, SyncFilePolicy> {
        &self.paths
    }
}

/// 固定脱敏原因，不含配置正文、宿主路径或内容。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SyncScopeError {
    /// 私有配置失效。
    #[error("同步私有配置已变化或不可验证")]
    ConfigurationChanged,
    /// 链、Vault、代次或实际配置摘要不一致。
    #[error("同步范围绑定不一致")]
    BindingMismatch,
    /// 包括旧基线路径在内，存在未批准路径或类型。
    #[error("同步范围未批准，必须协调范围变更")]
    ScopeChangeRequired,
    /// 观察没有显式覆盖完整联合路径。
    #[error("同步观察不完整")]
    Incomplete,
    /// 路径、前缀或重复记录有歧义。
    #[error("同步路径存在歧义或碰撞")]
    Collision,
    /// 输入或输出超过固定资源界限。
    #[error("同步范围超出预算")]
    Budget,
}
