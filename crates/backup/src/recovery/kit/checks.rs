//! 恢复写入前后的判定：前置检查、恢复必须造成的失效与核对事实、最终切换状态与双向同步门禁。
//!
//! 全部是已声明事实的纯判定，不做任何 IO；任一事实缺失都失败关闭。

use super::KitError;
use serde::{Deserialize, Serialize};

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

/// Auth 恢复必须造成的失效事实，以及随同恢复的 AgentStore／策略库必须满足的事实。
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
    /// 本次恢复是否包含策略库（`policy.sqlite3`）。
    pub policy_included: bool,
    /// 策略库的实例／类型／全局吊销已核对：快照截止点之后的吊销已重新生效，不会因回到旧快照而解除。
    pub policy_revocations_reconciled: bool,
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
        if !policy_revocations_settled(self.policy_included, self.policy_revocations_reconciled) {
            return Err(KitError::PolicyRevocationsNotReconciled);
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
    /// 本次恢复是否包含策略库（`policy.sqlite3`）。
    pub policy_included: bool,
    /// 策略库吊销已核对；纳入策略库而未核对时不得启用。
    pub policy_revocations_reconciled: bool,
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
            && policy_revocations_settled(self.policy_included, self.policy_revocations_reconciled)
    }
}

/// 纳入策略库时必须已核对吊销，未纳入则不作要求；恢复检查与启用门禁共用这一条规则。
fn policy_revocations_settled(included: bool, reconciled: bool) -> bool {
    !included || reconciled
}
