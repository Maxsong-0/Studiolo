//! 产物继承的不可拆分事实；批准来自可信启动记录，不能由 Runner 自报。
use crate::{BackupScope, DataLabels, SourcePolicyRef};
use serde::{Deserialize, Serialize};

/// 完整产物进入持久存储所继承的来源、标签、期限及原批准摘要。
/// 本值只验证事实形状；构造它不产生授权。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ArtifactProvenance {
    source_policy: SourcePolicyRef,
    labels: DataLabels,
    retain_until_unix_seconds: i64,
    approval_sha256: String,
}
/// 固定错误，不回显可能包含敏感内容的入参。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArtifactProvenanceError {
    /// 当前持久产物管线只接受明确的 Full 范围。
    #[error("产物没有完整持久化许可")]
    Scope,
    /// 期限须为正数且能无损表示为毫秒。
    #[error("产物留存期限无效")]
    Retention,
    /// 原批准必须绑定完整规范 SHA-256。
    #[error("产物批准摘要无效")]
    ApprovalDigest,
}
impl ArtifactProvenance {
    /// 固定可信 owner 提供的原事实，不读取配置、不检查当前时间或签发批准。
    pub fn new(
        source_policy: SourcePolicyRef,
        labels: DataLabels,
        retain_until_unix_seconds: i64,
        approval_sha256: impl Into<String>,
    ) -> Result<Self, ArtifactProvenanceError> {
        if labels.backup_scope() != BackupScope::Full {
            return Err(ArtifactProvenanceError::Scope);
        }
        if retain_until_unix_seconds <= 0 || retain_until_unix_seconds.checked_mul(1000).is_none() {
            return Err(ArtifactProvenanceError::Retention);
        }
        let approval_sha256 = approval_sha256.into();
        if !crate::is_sha256_hex(&approval_sha256) {
            return Err(ArtifactProvenanceError::ApprovalDigest);
        }
        Ok(Self {
            source_policy,
            labels,
            retain_until_unix_seconds,
            approval_sha256,
        })
    }
    /// 原逻辑来源及原生效策略。
    pub fn source_policy(&self) -> &SourcePolicyRef {
        &self.source_policy
    }
    /// 原批准的完整标签组；读取时不能重新分类。
    pub fn labels(&self) -> &DataLabels {
        &self.labels
    }
    /// 原期限，重放不能续期。
    pub fn retain_until_unix_seconds(&self) -> i64 {
        self.retain_until_unix_seconds
    }
    /// 可信配置批准内容的摘要。
    pub fn approval_sha256(&self) -> &str {
        &self.approval_sha256
    }
    /// 是否仍在原正文留存窗口内；负时间拒绝。
    pub fn is_live_at(&self, now_ms: i64) -> bool {
        now_ms >= 0 && now_ms < self.retain_until_unix_seconds * 1000
    }
}
impl<'de> Deserialize<'de> for ArtifactProvenance {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            source_policy: SourcePolicyRef,
            labels: DataLabels,
            retain_until_unix_seconds: i64,
            approval_sha256: String,
        }
        let fields = Fields::deserialize(deserializer)?;
        Self::new(
            fields.source_policy,
            fields.labels,
            fields.retain_until_unix_seconds,
            fields.approval_sha256,
        )
        .map_err(serde::de::Error::custom)
    }
}
