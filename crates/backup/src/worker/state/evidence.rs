//! 作业键与状态机依据的证据：上传证据、远端校验结果、未知阶段、失败原因与核对结论。

use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;
use studiolo_core::checkpoint::{CheckpointId, InstantMillis};

/// 作业键：单调序号加 checkpoint 标识。不用内容 hash 当作业键。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobKey {
    /// 单调递增的作业序号。
    pub sequence: NonZeroU64,
    /// 本作业封存的 checkpoint。
    pub checkpoint_id: CheckpointId,
}

/// 上传进程留下的证据。退出 0 本身不构成可恢复点。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadEvidence {
    /// 上传进程是否以 0 退出。
    pub exit_status_zero: bool,
    /// 上传进程自报的对象数；只是中间证据。
    pub reported_objects: u64,
}

/// 远端校验结果；三项全真才可能产生可恢复点。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteVerification {
    /// 远端对象是否完整。
    pub objects_complete: bool,
    /// 内容校验是否通过。
    pub checksum_verified: bool,
    /// 最终 manifest 是否成对引用了本次 checkpoint。
    pub paired_checkpoint_reference: bool,
    /// 远端校验通过的时刻。
    pub verified_at: InstantMillis,
}

impl RemoteVerification {
    /// 是否构成可恢复点的必要条件。
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        self.objects_complete && self.checksum_verified && self.paired_checkpoint_reference
    }
}

/// 结果未知发生在哪个阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownStage {
    /// 封存阶段结果未知。
    Sealing,
    /// 上传阶段结果未知（例如上传中断）。
    Uploading,
    /// 远端校验阶段结果未知。
    Verifying,
    /// 最终清单提交后 ACK 丢失。
    FinalManifestAck,
}

/// 失败原因；只记录可公开的固定类别，不带正文或秘密。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// 跨 owner 屏障没有成立。
    BarrierNotEstablished,
    /// 封存前来源发生变化。
    SourceChangedBeforeSeal,
    /// 磁盘空间不足。
    OutOfSpace,
    /// 上传被拒绝或明确失败。
    UploadRejected,
    /// 远端校验不通过。
    RemoteVerificationFailed,
    /// 最终清单提交失败。
    FinalManifestCommitFailed,
    /// 备份目标未接入，无法验证。
    TargetUnverified,
}

/// 未知结果沿原作业键核对后的结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ReconciledOutcome {
    /// 远端确认没有该作业键的对象。
    RemoteAbsent,
    /// 远端确认存在且通过校验。
    RemoteVerified {
        /// 远端校验结果。
        verification: RemoteVerification,
    },
}
