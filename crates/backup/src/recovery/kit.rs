//! recovery kit：仅本机 TTY/localhost 的一次性、有期限恢复身份。
//!
//! kit 独立加密保存，不进入普通 checkpoint。任何恢复写入之前先建立 transcript；
//! 先在同步与真实执行 Adapter 禁用下完成前置检查，再恢复主人与密钥映射。

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

/// 恢复写入前必须完成的检查；同步与真实执行 Adapter 必须先禁用。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreRestoreChecks {
    /// 同步 Adapter 已禁用。
    pub sync_adapter_disabled: bool,
    /// 真实执行 Adapter 已禁用。
    pub execution_adapter_disabled: bool,
    /// 内容已检查。
    pub content_verified: bool,
    /// 版本已检查。
    pub version_verified: bool,
    /// 权限已检查。
    pub permissions_verified: bool,
    /// 哈希已检查。
    pub hashes_verified: bool,
    /// 审计连续性已检查。
    pub audit_continuity_verified: bool,
    /// 空间已检查。
    pub space_verified: bool,
}

impl PreRestoreChecks {
    /// 全部通过才允许恢复主人与密钥映射。
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.sync_adapter_disabled
            && self.execution_adapter_disabled
            && self.content_verified
            && self.version_verified
            && self.permissions_verified
            && self.hashes_verified
            && self.audit_continuity_verified
            && self.space_verified
    }
}

/// Auth 恢复必须造成的失效事实。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthRestoreInvalidation {
    /// 旧会话已失效。
    pub old_sessions_invalidated: bool,
    /// 旧 L3 已失效。
    pub l3_invalidated: bool,
    /// 旧快照里的恢复码不能直接重新生效。
    pub snapshot_recovery_codes_blocked: bool,
    /// 已消费记录仍然保留，没有靠删表绕过。
    pub consumed_records_preserved: bool,
    /// 审计锚点仍然保留。
    pub audit_anchor_preserved: bool,
    /// 保持原主人绑定。
    pub owner_binding_preserved: bool,
    /// 已重新认证。
    pub reauthenticated: bool,
    /// 新的签发代次。
    pub issuance_generation: u64,
    /// 本次恢复是否包含 AgentStore。
    pub agent_store_included: bool,
    /// AgentStore 旧类型 token 已失效。
    pub agent_type_tokens_invalidated: bool,
    /// AgentStore 旧实例已失效。
    pub agent_instances_invalidated: bool,
}

impl AuthRestoreInvalidation {
    /// 校验失效事实；签发代次必须严格大于旧快照的代次。
    pub fn check(&self, previous_generation: u64) -> Result<(), KitError> {
        if !(self.old_sessions_invalidated && self.l3_invalidated) {
            return Err(KitError::OldCredentialsStillValid);
        }
        if !self.snapshot_recovery_codes_blocked {
            return Err(KitError::SnapshotRecoveryCodeReusable);
        }
        if !(self.consumed_records_preserved && self.audit_anchor_preserved) {
            return Err(KitError::ConsumedRecordsDropped);
        }
        if !(self.owner_binding_preserved && self.reauthenticated) {
            return Err(KitError::OwnerBindingNotReauthenticated);
        }
        if self.issuance_generation <= previous_generation {
            return Err(KitError::IssuanceGenerationNotAdvanced);
        }
        if self.agent_store_included
            && !(self.agent_type_tokens_invalidated && self.agent_instances_invalidated)
        {
            return Err(KitError::AgentStoreCredentialsStillValid);
        }
        Ok(())
    }
}

/// 最终切换的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutoverState {
    /// 只记了 Intent，尚未切换。
    IntentRecorded,
    /// 已切换并真实记录 Outcome。
    Completed,
    /// 已回滚。
    RolledBack,
    /// 保持只读停服。
    ReadOnlyHalted,
}

/// 启用双向同步前必须成立的门禁。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BidirectionalSyncGate {
    /// 桌面与恢复点的差异已隔离。
    pub divergence_isolated: bool,
    /// 差异已人工协调。
    pub manually_reconciled: bool,
    /// 新 checkpoint 已验收。
    pub new_checkpoint_accepted: bool,
    /// 写入与备份链已重新验收。
    pub write_and_backup_chain_reaccepted: bool,
    /// 旧目录仍保留至回滚窗口结束。
    pub old_root_retained: bool,
}

impl BidirectionalSyncGate {
    /// 是否可以启用双向同步。
    #[must_use]
    pub fn may_enable(&self) -> bool {
        self.divergence_isolated
            && self.manually_reconciled
            && self.new_checkpoint_accepted
            && self.write_and_backup_chain_reaccepted
            && self.old_root_retained
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
        }
    }
}

impl std::error::Error for KitError {}
