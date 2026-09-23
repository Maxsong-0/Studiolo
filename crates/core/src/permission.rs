//! 逻辑工具名 `ToolId` 与只提供收窄操作的权限集合 `PermissionSet`。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

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
