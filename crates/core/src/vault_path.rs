//! Vault 内的相对路径 `VaultPath` 及其纯语法校验；全程不访问文件系统。

use std::{
    fmt,
    path::{Component, Path},
    str::FromStr,
};

use serde::{Deserialize, Serialize};

use crate::PathSyntaxError;

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

// VaultPath 的顺序与内部字符串一致，因此可以按逻辑前缀借用查询有序映射。
impl std::borrow::Borrow<str> for VaultPath {
    fn borrow(&self) -> &str {
        self.as_str()
    }
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
