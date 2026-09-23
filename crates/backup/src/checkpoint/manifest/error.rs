//! 清单组装与校验的失败模式。

use studiolo_core::checkpoint::CheckpointOwner;

/// 清单组装与校验的失败模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// 已装配 owner 缺少自己的水位。
    WatermarkMissing {
        /// 缺失水位的 owner。
        owner: CheckpointOwner,
    },
    /// 水位字段被别的计数顶替（例如索引 generation 或退出时间戳）。
    WatermarkSubstituted {
        /// 被顶替水位的 owner。
        owner: CheckpointOwner,
        /// 被顶替的字段名。
        field: &'static str,
    },
    /// 水位字段格式不合法。
    WatermarkMalformed {
        /// 字段所属 owner。
        owner: CheckpointOwner,
        /// 字段名。
        field: &'static str,
    },
    /// Broker 仍有未决操作。
    BrokerUnresolved,
    /// Control outbox 仍有未投递条目且投递水位为 0。
    ControlOutboxUnsettled,
    /// Bridge 持久门禁未关闭或观察序号为 0。
    BridgeGateOpen,
    /// 缺少固定应用或 CLI 包摘要。
    PackageDigestsMissing,
    /// 缺少部署清单。
    DeploymentManifestMissing,
    /// 秘密引用不是纯粹的「去哪拿」。
    SecretReferenceMalformed,
    /// 已 ACK 事件没有逐条对应原事件。
    AuditCoverageNotExact,
    /// 清单 schema 版本未知。
    UnsupportedSchema {
        /// 读到的版本号。
        found: u32,
    },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WatermarkMissing { owner } => {
                write!(formatter, "owner {} 缺少水位", owner.as_str())
            }
            Self::WatermarkSubstituted { owner, field } => write!(
                formatter,
                "owner {} 的 {field} 被其他计数顶替",
                owner.as_str()
            ),
            Self::WatermarkMalformed { owner, field } => {
                write!(formatter, "owner {} 的 {field} 格式不合法", owner.as_str())
            }
            Self::BrokerUnresolved => formatter.write_str("Broker 仍有未决操作"),
            Self::ControlOutboxUnsettled => formatter.write_str("Control outbox 状态未结清"),
            Self::BridgeGateOpen => formatter.write_str("Bridge 持久门禁未关闭"),
            Self::PackageDigestsMissing => formatter.write_str("缺少固定应用或 CLI 包摘要"),
            Self::DeploymentManifestMissing => formatter.write_str("缺少部署清单"),
            Self::SecretReferenceMalformed => formatter.write_str("秘密引用不是脱敏的来源映射"),
            Self::AuditCoverageNotExact => {
                formatter.write_str("已 ACK 事件没有逐条对应 Audit 截止点内原事件")
            }
            Self::UnsupportedSchema { found } => write!(formatter, "清单 schema 版本 {found} 未知"),
        }
    }
}

impl std::error::Error for ManifestError {}
