//! 任务、执行尝试与 Vault 的稳定标识符：ULID／UUID 新类型及其文本形式。

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use ulid::Ulid;
use uuid::Uuid;

use crate::IdParseError;

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
