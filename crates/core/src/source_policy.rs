//! 原逻辑来源和生效策略的引用；形状合法不等于获得主人批准。
use serde::{Deserialize, Serialize};

/// 内容标签继承的原策略身份；可信 owner 提供，不能把 frontmatter 自报当授权。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourcePolicyRef {
    source_id: String,
    effective_policy_sha256: String,
}
/// 无自由文本错误，避免把不合法引用中的敏感内容写进日志。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SourcePolicyError {
    /// 逻辑来源必须是有界且无路径语义的名字。
    #[error("来源逻辑名无效")]
    InvalidSource,
    /// 策略必须绑定完整规范 SHA-256，不能填版本号、路径或 latest。
    #[error("来源策略摘要无效")]
    InvalidPolicyDigest,
}
impl SourcePolicyRef {
    /// 接受固定逻辑名和实际策略摘要，不读取配置、不批准备份或模型外发。
    pub fn new(
        source_id: impl Into<String>,
        effective_policy_sha256: impl Into<String>,
    ) -> Result<Self, SourcePolicyError> {
        let source_id = source_id.into();
        if source_id.is_empty()
            || source_id.len() > 128
            || !source_id.as_bytes()[0].is_ascii_alphanumeric()
            || !source_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            || source_id.contains("..")
        {
            return Err(SourcePolicyError::InvalidSource);
        }
        let effective_policy_sha256 = effective_policy_sha256.into();
        if effective_policy_sha256.len() != 64
            || !effective_policy_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(SourcePolicyError::InvalidPolicyDigest);
        }
        Ok(Self {
            source_id,
            effective_policy_sha256,
        })
    }
    /// 固定来源逻辑名，不能用作文件路径。
    pub fn source_id(&self) -> &str {
        &self.source_id
    }
    /// 原生效策略内容的完整 SHA-256。
    pub fn effective_policy_sha256(&self) -> &str {
        &self.effective_policy_sha256
    }
}
impl<'de> Deserialize<'de> for SourcePolicyRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            source_id: String,
            effective_policy_sha256: String,
        }
        let fields = Fields::deserialize(deserializer)?;
        Self::new(fields.source_id, fields.effective_policy_sha256)
            .map_err(serde::de::Error::custom)
    }
}
