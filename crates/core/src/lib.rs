//! Studiolo 各模块接缝处共用的值类型与端口。

use std::{
    collections::BTreeSet,
    fmt,
    num::NonZeroU64,
    path::{Component, Path},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
pub mod checkpoint;
mod revision_hasher;
pub use revision_hasher::RevisionHasher;
use ulid::Ulid;
use uuid::Uuid;

/// 文本形式的标识符格式不合法时返回的错误。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind}: {value}")]
pub struct IdParseError {
    kind: &'static str,
    value: String,
}

/// 任务的稳定标识符。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(Ulid);

impl TaskId {
    /// 生成一个新的任务标识符。
    #[must_use]
    pub fn new() -> Self {
        Self(Ulid::generate())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for TaskId {
    type Err = IdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ulid::from_string(value)
            .map(Self)
            .map_err(|_| IdParseError {
                kind: "task id",
                value: value.to_owned(),
            })
    }
}

/// 单次执行尝试（attempt）的稳定标识符。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AttemptId(Ulid);

impl AttemptId {
    /// 生成一个新的尝试标识符。
    #[must_use]
    pub fn new() -> Self {
        Self(Ulid::generate())
    }
}

impl Default for AttemptId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AttemptId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "A-{}", self.0)
    }
}

impl FromStr for AttemptId {
    type Err = IdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let raw = value.strip_prefix("A-").ok_or_else(|| IdParseError {
            kind: "attempt id",
            value: value.to_owned(),
        })?;
        Ulid::from_string(raw).map(Self).map_err(|_| IdParseError {
            kind: "attempt id",
            value: value.to_owned(),
        })
    }
}

impl TryFrom<String> for AttemptId {
    type Error = IdParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<AttemptId> for String {
    fn from(value: AttemptId) -> Self {
        value.to_string()
    }
}

/// 单个规范 Vault 的持久身份。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VaultUuid(Uuid);

impl VaultUuid {
    /// 生成一个全新的 Vault 身份。
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for VaultUuid {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for VaultUuid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for VaultUuid {
    type Err = IdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self).map_err(|_| IdParseError {
            kind: "vault uuid",
            value: value.to_owned(),
        })
    }
}

/// 相对 Vault 路径在任何文件系统访问之前就被拒绝的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathSyntaxError {
    /// 路径为空。
    #[error("path is empty")]
    Empty,
    /// 路径是绝对路径，或含空分量、`.` 或 `..` 分量。
    #[error("path is not a clean relative path")]
    NotRelative,
    /// 路径含反斜杠、控制字符（含 NUL）、组合附加符号、Windows 盘符前缀（如 `C:/`），
    /// 或百分号编码的分隔符／点／NUL（`%2f`、`%5c`、`%2e`、`%00`），或任何非 Unicode NFC 的拼写。
    #[error("path contains a forbidden encoding or character")]
    ForbiddenEncoding,
}

/// 相对于已批准 Vault 根目录、语法上安全的路径。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct VaultPath(String);

impl VaultPath {
    /// 只做语法解析，全程不访问环境文件系统。
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, PathSyntaxError> {
        let raw = raw.as_ref();
        if raw.is_empty() {
            return Err(PathSyntaxError::Empty);
        }
        let lower = raw.to_ascii_lowercase();
        let bytes = raw.as_bytes();
        let has_windows_drive_prefix = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/';
        // 只接受 NFC 拼写：分解形式（含日文浊音符、Hangul jamo 等黑名单外的形态）与
        // 组合形式在归一化不敏感的文件系统上指向同一物理文件，会造成两套逻辑身份。
        if raw.contains('\\')
            || raw.chars().any(char::is_control)
            || raw.chars().any(is_unicode_combining_mark)
            || !unicode_normalization::is_nfc(raw)
            || has_windows_drive_prefix
            || lower.contains("%2f")
            || lower.contains("%5c")
            || lower.contains("%00")
            || lower.contains("%2e")
        {
            return Err(PathSyntaxError::ForbiddenEncoding);
        }
        let path = Path::new(raw);
        if path.is_absolute()
            || raw
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | ".."))
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::Prefix(_)
                        | Component::RootDir
                        | Component::CurDir
                        | Component::ParentDir
                )
            })
        {
            return Err(PathSyntaxError::NotRelative);
        }
        Ok(Self(raw.to_owned()))
    }

    /// 返回原始的相对路径。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 返回普通调用方是否被禁止改写此路径。
    #[must_use]
    pub fn is_protected(&self) -> bool {
        let folded = self.0.to_ascii_lowercase();
        let first = folded.split('/').next().unwrap_or_default();
        matches!(first, ".git" | ".workbench" | ".obsidian") || folded == ".stignore"
    }

    /// 返回小写折叠后的键，供保守的撞名检查使用。解析得到的路径已是 NFC，
    /// 因此只做折叠即可与经 NFC 归一化的目录项名相比较。
    #[must_use]
    pub fn collision_key(&self) -> String {
        self.0.to_lowercase()
    }
}

fn is_unicode_combining_mark(character: char) -> bool {
    matches!(
        character as u32,
        0x0300..=0x036f
            | 0x1ab0..=0x1aff
            | 0x1dc0..=0x1dff
            | 0x20d0..=0x20ff
            | 0xfe20..=0xfe2f
    )
}

impl fmt::Display for VaultPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for VaultPath {
    type Err = PathSyntaxError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for VaultPath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// 绑定到 Vault 身份与逻辑路径的内容修订值（Revision）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision([u8; 32]);

impl Revision {
    /// 计算 Vault 中某路径上内容的 SHA-256 修订值。
    #[must_use]
    pub fn compute(vault: VaultUuid, path: &VaultPath, content: &[u8]) -> Self {
        let mut hasher = RevisionHasher::new(vault, path);
        hasher.update(content);
        hasher.finish()
    }

    /// 返回小写十六进制摘要。
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut output = String::with_capacity(64);
        for byte in self.0 {
            use fmt::Write as _;
            write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
        }
        output
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

/// 单调递增的令牌，用来把一次尝试与更早的尝试隔离开（fencing）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fence(NonZeroU64);

impl Fence {
    /// 构造非零的 fence；传入 0 时返回 `None`。
    #[must_use]
    pub fn new(value: u64) -> Option<Self> {
        NonZeroU64::new(value).map(Self)
    }

    /// 返回 fence 的数值。
    #[must_use]
    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// 稳定的逻辑工具名，例如 `web.fetch`。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ToolId(String);

impl ToolId {
    /// 由编译期常量构造工具标识符；值为空或含控制字符时直接恐慌。
    #[must_use]
    pub fn from_static(value: &'static str) -> Self {
        assert!(
            !value.is_empty() && !value.chars().any(char::is_control),
            "invalid tool identifier"
        );
        Self(value.to_owned())
    }

    /// 返回逻辑工具名。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ToolId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(serde::de::Error::custom("invalid tool identifier"));
        }
        Ok(Self(value))
    }
}

/// 权限集合：对外只提供收窄操作，从不提供放宽操作。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PermissionSet(BTreeSet<ToolId>);

impl PermissionSet {
    /// 由显式授予的工具构造权限集合。
    #[must_use]
    pub fn new(tools: impl IntoIterator<Item = ToolId>) -> Self {
        Self(tools.into_iter().collect())
    }

    /// 返回两个独立施加的限制的交集。
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        Self(self.0.intersection(&other.0).cloned().collect())
    }

    /// 返回某工具是否在生效集合中。
    #[must_use]
    pub fn allows(&self, tool: &ToolId) -> bool {
        self.0.contains(tool)
    }
}

/// 源内容的敏感分级。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    /// 公开数据。
    Public,
    /// 内部数据。
    Internal,
    /// 机密数据。
    Confidential,
    /// 受限数据，适用最严格的默认拒绝规则。
    Restricted,
}

/// 源材料中可进入持久备份的范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupScope {
    /// 可备份经校验的完整对象。
    Full,
    /// 只可备份脱敏摘要与清单（manifest）。
    SummaryOnly,
    /// 只可备份不含正文的墓碑记录（tombstone）。
    None,
}

/// 源材料中可同步到桌面端的范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopSync {
    /// 不同步任何内容。
    Deny,
    /// 只同步脱敏摘要。
    SummaryOnly,
}

/// 源材料能否发送给模型。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEgress {
    /// 禁止向模型外发。
    Deny,
    /// 只允许经批准的本地模型读取。
    LocalOnly,
    /// 只允许列明的提供方读取。
    AllowProviders(BTreeSet<String>),
}

/// 源标签组合自相矛盾时返回的错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DataLabelError {
    /// 被排除在备份之外的内容不得同步到桌面端。
    #[error("backup_scope=none requires desktop_sync=deny")]
    DesktopOutlivesBackup,
    /// 受限内容不得批准发给云端模型提供方。
    #[error("restricted content cannot allow a cloud model provider")]
    RestrictedProviderEgress,
    /// 提供方白名单至少要列出一个提供方。
    #[error("model provider allow-list is empty")]
    EmptyProviderList,
}

/// 经校验、由对象继承的分级、留存与外发标签。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DataLabels {
    classification: Classification,
    backup_scope: BackupScope,
    desktop_sync: DesktopSync,
    model_egress: ModelEgress,
}

impl<'de> Deserialize<'de> for DataLabels {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            classification: Classification,
            backup_scope: BackupScope,
            desktop_sync: DesktopSync,
            model_egress: ModelEgress,
        }
        let fields = Fields::deserialize(deserializer)?;
        Self::new(
            fields.classification,
            fields.backup_scope,
            fields.desktop_sync,
            fields.model_egress,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl DataLabels {
    /// 校验并构造一组不可拆分的标签。
    pub fn new(
        classification: Classification,
        backup_scope: BackupScope,
        desktop_sync: DesktopSync,
        model_egress: ModelEgress,
    ) -> Result<Self, DataLabelError> {
        if backup_scope == BackupScope::None && desktop_sync != DesktopSync::Deny {
            return Err(DataLabelError::DesktopOutlivesBackup);
        }
        if matches!(&model_egress, ModelEgress::AllowProviders(providers) if providers.is_empty()) {
            return Err(DataLabelError::EmptyProviderList);
        }
        if classification == Classification::Restricted
            && matches!(model_egress, ModelEgress::AllowProviders(_))
        {
            return Err(DataLabelError::RestrictedProviderEgress);
        }
        Ok(Self {
            classification,
            backup_scope,
            desktop_sync,
            model_egress,
        })
    }

    /// 返回敏感分级。
    #[must_use]
    pub fn classification(&self) -> Classification {
        self.classification
    }

    /// 返回备份范围。
    #[must_use]
    pub fn backup_scope(&self) -> BackupScope {
        self.backup_scope
    }

    /// 返回桌面同步范围。
    #[must_use]
    pub fn desktop_sync(&self) -> DesktopSync {
        self.desktop_sync
    }

    /// 返回模型外发规则。
    #[must_use]
    pub fn model_egress(&self) -> &ModelEgress {
        &self.model_egress
    }
}
