use super::{FinalManifestDraft, FinalManifestError};
use crate::worker::restic::SnapshotRef;
use serde::{Deserialize, Serialize};
use studiolo_core::checkpoint::InstantMillis;

/// 远端已读回的最终记录引用；只有真实提交适配器可以提供这种事实，不能以本地写入替代。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FinalManifestReceipt(Fields);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    draft: FinalManifestDraft,
    final_snapshot: SnapshotRef,
    record_sha256: String,
    finalized_at: InstantMillis,
}
impl FinalManifestReceipt {
    /// 提交适配器在实际读回完整记录并核验摘要后调用；不能把此 DTO 构造当作远端 IO 证明。
    pub fn new(
        draft: FinalManifestDraft,
        final_snapshot: SnapshotRef,
        finalized_at: InstantMillis,
    ) -> Result<Self, FinalManifestError> {
        let record_sha256 = draft.sha256()?;
        let receipt = Self(Fields {
            draft,
            final_snapshot,
            record_sha256,
            finalized_at,
        });
        receipt.validate()?;
        Ok(receipt)
    }
    fn validate(&self) -> Result<(), FinalManifestError> {
        if self.0.final_snapshot.as_str().len() != 64
            || &self.0.final_snapshot == self.0.draft.data_snapshot()
            || self.0.record_sha256 != self.0.draft.sha256()?
            || self.0.finalized_at < self.0.draft.data_verified_at()
        {
            return Err(FinalManifestError::Invalid);
        }
        Ok(())
    }
    /// 已逐字节核验的确定最终记录。
    #[must_use]
    pub fn draft(&self) -> &FinalManifestDraft {
        &self.0.draft
    }
    /// 最终记录所在的独立远端快照。
    #[must_use]
    pub fn final_snapshot(&self) -> &SnapshotRef {
        &self.0.final_snapshot
    }
    /// 最终记录实际读回复验完成的时刻；不用于替代内容截止时刻。
    #[must_use]
    pub fn finalized_at(&self) -> InstantMillis {
        self.0.finalized_at
    }
}
impl<'de> Deserialize<'de> for FinalManifestReceipt {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Self(Fields::deserialize(deserializer)?);
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}
