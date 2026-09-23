//! recovery kit：仅本机 TTY/localhost 的一次性、有期限恢复身份。
//!
//! kit 独立加密保存，不进入普通 checkpoint。任何恢复写入之前先建立 transcript；
//! 先在同步与真实执行 Adapter 禁用下完成前置检查，再恢复主人与密钥映射。
//! 前置检查、失效与核对事实、启用门禁在 `checks` 子模块，仍经本模块原路径导出。

mod checks;

pub use checks::{AuthRestoreInvalidation, BidirectionalSyncGate, CutoverState, PreRestoreChecks};

use super::transcript::{RecoveryTranscript, TranscriptError, WriteAuthorization};
use crate::checkpoint::manifest::SecretReference;
use serde::{Deserialize, Serialize};
use studiolo_core::checkpoint::{CheckpointId, InstantMillis};

/// restore bootstrap 的来源通道。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapChannel {
    /// 本机控制终端。
    LocalTty,
    /// 本机回环地址。
    Loopback,
    /// 任何非本机来源。
    RemoteNetwork,
}

/// 一次性、有期限的恢复身份。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryIdentity {
    /// 身份标识；不是凭据本体。
    pub identity_id: String,
    /// 过期时刻。
    pub expires_at: InstantMillis,
    /// 是否已被使用。
    pub redeemed: bool,
}

/// 恢复 kit 的固定内容说明；这里只有「有什么、去哪拿」，没有秘密值。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryKit {
    /// 对应的 checkpoint。
    pub checkpoint_id: CheckpointId,
    /// 一次性恢复身份。
    pub identity: RecoveryIdentity,
    /// 固定包摘要（64 位小写十六进制）。
    pub package_digest: String,
    /// 主人与秘密映射的脱敏引用。
    pub secret_references: Vec<SecretReference>,
    /// 是否包含 Sync 端到端恢复资料。
    pub sync_recovery_material: bool,
    /// kit 是否独立加密保存在普通 checkpoint 之外。
    pub stored_outside_ordinary_checkpoint: bool,
}

/// 兑换成功后的恢复授权。
#[derive(Debug)]
pub struct RecoveryGrant {
    /// 已建立的 transcript。
    pub transcript: RecoveryTranscript,
    /// 恢复写入授权。
    pub authorization: WriteAuthorization,
}

impl RecoveryKit {
    /// 兑换恢复身份：只允许本机 TTY/localhost，一次性且有期限。
    ///
    /// 兑换成功即建立 transcript，并在其中记下第一条 `KitOpened`。
    pub fn redeem(
        &mut self,
        channel: BootstrapChannel,
        now: InstantMillis,
    ) -> Result<RecoveryGrant, KitError> {
        if channel == BootstrapChannel::RemoteNetwork {
            return Err(KitError::RemoteBootstrapForbidden);
        }
        if !self.stored_outside_ordinary_checkpoint {
            return Err(KitError::KitInsideOrdinaryCheckpoint);
        }
        if self.identity.redeemed {
            return Err(KitError::IdentityAlreadyRedeemed);
        }
        if now > self.identity.expires_at {
            return Err(KitError::IdentityExpired);
        }
        self.identity.redeemed = true;
        let mut transcript = RecoveryTranscript::begin(self.checkpoint_id.clone());
        transcript
            .append(
                super::transcript::RecoveryStep::KitOpened,
                serde_json::json!({
                    "identity_id": self.identity.identity_id,
                    "channel": channel,
                    "package_digest": self.package_digest,
                }),
            )
            .map_err(KitError::Transcript)?;
        let authorization = transcript.authorize_write().map_err(KitError::Transcript)?;
        Ok(RecoveryGrant {
            transcript,
            authorization,
        })
    }
}

/// recovery kit 的失败模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KitError {
    /// 非本机来源不得做 restore bootstrap。
    RemoteBootstrapForbidden,
    /// kit 被放进了普通 checkpoint。
    KitInsideOrdinaryCheckpoint,
    /// 一次性身份已被使用。
    IdentityAlreadyRedeemed,
    /// 恢复身份已过期。
    IdentityExpired,
    /// 旧会话或 L3 未失效。
    OldCredentialsStillValid,
    /// 旧快照里的恢复码仍可直接生效。
    SnapshotRecoveryCodeReusable,
    /// 已消费记录或审计锚点被删表绕过。
    ConsumedRecordsDropped,
    /// 主人绑定未保持或未重新认证。
    OwnerBindingNotReauthenticated,
    /// 签发代次没有前进，旧序号可能复用旧 `event_id`。
    IssuanceGenerationNotAdvanced,
    /// AgentStore 旧类型 token 或实例仍然有效。
    AgentStoreCredentialsStillValid,
    /// 策略库恢复后未核对吊销，旧快照可能让已吊销的实例或类型复活。
    PolicyRevocationsNotReconciled,
    /// transcript 相关失败。
    Transcript(TranscriptError),
}

impl std::fmt::Display for KitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transcript(error) => write!(formatter, "恢复 transcript 失败：{error}"),
            Self::RemoteBootstrapForbidden => {
                formatter.write_str("restore bootstrap 只允许本机 TTY/localhost")
            }
            Self::KitInsideOrdinaryCheckpoint => {
                formatter.write_str("recovery kit 不得进入普通 checkpoint")
            }
            Self::IdentityAlreadyRedeemed => formatter.write_str("一次性恢复身份已被使用"),
            Self::IdentityExpired => formatter.write_str("恢复身份已过期"),
            Self::OldCredentialsStillValid => formatter.write_str("旧会话或 L3 未失效"),
            Self::SnapshotRecoveryCodeReusable => formatter.write_str("旧快照恢复码仍可直接生效"),
            Self::ConsumedRecordsDropped => formatter.write_str("已消费记录或审计锚点被绕过"),
            Self::OwnerBindingNotReauthenticated => {
                formatter.write_str("主人绑定未保持或未重新认证")
            }
            Self::IssuanceGenerationNotAdvanced => formatter.write_str("签发代次没有前进"),
            Self::AgentStoreCredentialsStillValid => {
                formatter.write_str("AgentStore 旧类型 token 或实例仍有效")
            }
            Self::PolicyRevocationsNotReconciled => {
                formatter.write_str("策略库恢复后未核对吊销：旧快照可能让已吊销的实例或类型复活")
            }
        }
    }
}

impl std::error::Error for KitError {}
