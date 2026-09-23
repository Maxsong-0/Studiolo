//! 可变参数的封闭检查：仓库定位符、作业 tag、快照标识、绝对路径与封闭字符集。

use super::ResticError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 备份仓库定位符；只接受绝对路径或固定前缀的远端形式。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepositoryRef(String);

impl RepositoryRef {
    /// 解析仓库定位符。
    pub fn parse(value: impl Into<String>) -> Result<Self, ResticError> {
        let value = value.into();
        if !is_closed_token(&value) || value.len() > 512 {
            return Err(ResticError::ForbiddenArgument);
        }
        let accepted = value.starts_with('/')
            || value.starts_with("rest:https://")
            || value.starts_with("sftp:")
            || value.starts_with("s3:https://");
        if !accepted {
            return Err(ResticError::ForbiddenArgument);
        }
        Ok(Self(value))
    }

    /// 返回定位符文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 作业 tag；只允许小写字母、数字、点、下划线与连字符。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobTag(String);

impl JobTag {
    /// 解析 tag。
    pub fn parse(value: impl Into<String>) -> Result<Self, ResticError> {
        let value = value.into();
        let shaped = !value.is_empty()
            && value.len() <= 64
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            });
        if shaped {
            Ok(Self(value))
        } else {
            Err(ResticError::ForbiddenArgument)
        }
    }

    /// 返回 tag 文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 远端快照标识；8 至 64 位小写十六进制。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotRef(String);

impl SnapshotRef {
    /// 解析快照标识。
    pub fn parse(value: impl Into<String>) -> Result<Self, ResticError> {
        let value = value.into();
        let shaped = (8..=64).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if shaped {
            Ok(Self(value))
        } else {
            Err(ResticError::ForbiddenArgument)
        }
    }

    /// 返回快照标识文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub(super) fn absolute_argument(path: &Path) -> Result<String, ResticError> {
    let text = path.to_str().ok_or(ResticError::ForbiddenArgument)?;
    if !path.is_absolute() || text.contains("/../") || text.ends_with("/..") {
        return Err(ResticError::ForbiddenArgument);
    }
    Ok(text.to_owned())
}

/// 封闭字符集：控制字符、空白与 shell 元字符一律拒绝。
pub(super) fn is_closed_token(value: &str) -> bool {
    !value.is_empty()
        && !value.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(
                    character,
                    ';' | '&'
                        | '|'
                        | '$'
                        | '`'
                        | '<'
                        | '>'
                        | '('
                        | ')'
                        | '{'
                        | '}'
                        | '['
                        | ']'
                        | '*'
                        | '?'
                        | '!'
                        | '\\'
                        | '\''
                        | '"'
                        | '\u{0}'
                )
        })
}
