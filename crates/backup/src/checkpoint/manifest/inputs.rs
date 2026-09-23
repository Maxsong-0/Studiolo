//! 清单里不属于任何单个 SQLite owner 的固定事实：包摘要、部署清单、脱敏秘密引用
//! 与已 ACK 事件的逐条对应证明。

use super::ManifestError;
use serde::{Deserialize, Serialize};

/// 固定应用或 CLI 包的摘要。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageDigest {
    /// 包名。
    pub name: String,
    /// 固定版本号。
    pub version: String,
    /// 内容摘要（64 位小写十六进制）。
    pub digest: String,
}

/// 部署清单的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentEntry {
    /// 部署单元名。
    pub unit: String,
    /// 该单元的固定 revision。
    pub revision: String,
}

/// 秘密的来源；只保留「去哪拿」，永不保留值本身。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecretSource {
    /// 从环境变量注入；这里只记录变量名。
    EnvironmentVariable {
        /// 变量名，必须是大写、下划线与数字。
        variable: String,
    },
}

/// 脱敏后的秘密引用映射。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretReference {
    /// 逻辑名，例如 `restic_repository_password`。
    pub logical_name: String,
    /// 去哪拿。
    pub source: SecretSource,
}

/// 已 ACK 事件逐条对应 Audit 截止点内原事件的证明。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditCoverageProof {
    /// 本次逐条核对的已 ACK 事件数。
    pub acked_events_checked: u64,
    /// 其中精确对应到原事件的条数。
    pub acked_events_matched: u64,
    /// 是否只比较了 offset 小于最后 offset；true 表示证明不成立。
    pub offset_comparison_only: bool,
}

impl AuditCoverageProof {
    /// 只有逐条对应且不是纯 offset 比较才算成立。
    #[must_use]
    pub fn is_exact(&self) -> bool {
        !self.offset_comparison_only
            && self.acked_events_checked > 0
            && self.acked_events_checked == self.acked_events_matched
    }
}

/// 清单构建时需要、但不属于任何单个 SQLite owner 的固定事实。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestInputs {
    /// 固定应用包摘要。
    pub application_packages: Vec<PackageDigest>,
    /// 固定 CLI 包摘要。
    pub cli_packages: Vec<PackageDigest>,
    /// 部署清单。
    pub deployment: Vec<DeploymentEntry>,
    /// 脱敏秘密引用映射。
    pub secret_references: Vec<SecretReference>,
    /// 已 ACK 事件逐条对应证明。
    pub audit_coverage: AuditCoverageProof,
}

/// 秘密引用只能是「去哪拿」：逻辑名非空且有界，环境变量名只含大写、数字与下划线。
pub(super) fn check_secret_reference(reference: &SecretReference) -> Result<(), ManifestError> {
    if reference.logical_name.trim().is_empty() || reference.logical_name.len() > 128 {
        return Err(ManifestError::SecretReferenceMalformed);
    }
    let SecretSource::EnvironmentVariable { variable } = &reference.source;
    let shaped = !variable.is_empty()
        && variable.len() <= 128
        && variable
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        && !variable.starts_with(|c: char| c.is_ascii_digit());
    if !shaped {
        return Err(ManifestError::SecretReferenceMalformed);
    }
    Ok(())
}
