//! 可信 owner 提供来源，Broker 固定首次请求的元数据；它不是 HTTP 授权凭据。

pub(crate) mod pending;
pub(crate) use pending::cancel_for_maintenance;
mod replay;
mod restore;
pub(crate) mod store;
pub use pending::PendingRecordedRestore;
pub(crate) mod capacity;
pub(crate) mod envelope;
mod validation;
pub use restore::RecordedRestore;
pub(crate) use validation::{check_existing, validate_terminal};

use crate::{BrokerError, Revision};
use serde::{Deserialize, Serialize};

/// 已认证的保存主体或真实摄入通道；不猜具体 Obsidian 设备或个人。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MutationActor {
    /// issuer 与 subject 由本次可信会话产生。
    Human {
        /// 本次可信认证的签发方标识。
        issuer: String,
        /// 签发方给出的主体标识，不从客户端显示名称推导。
        subject: String,
    },
    /// 受信 bridge 观察到的外部 Obsidian 变更。
    Obsidian,
}

/// 固定的正文操作类型；恢复来源是路径绑定的历史 revision。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MutationKind {
    /// 普通正文保存或 bridge 摄入。
    Replace,
    /// owner 已经验证历史正文、当前范围、生成区、本次身份和审计准入。
    Restore {
        /// 同一逻辑路径下作为恢复来源的历史 revision。
        source_revision: Revision,
    },
}

/// 不从 JSON 反序列化，也不允许客户端提交时间；可信调用方仍须独立授权。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutationMetadata {
    pub(crate) actor: MutationActor,
    pub(crate) kind: MutationKind,
}

impl MutationMetadata {
    /// 验证固定主体字段，不创建或缓存授权。
    pub fn new(actor: MutationActor, kind: MutationKind) -> Result<Self, BrokerError> {
        if let MutationActor::Human { issuer, subject } = &actor {
            for (value, maximum) in [(issuer, 512), (subject, 256)] {
                if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control)
                {
                    return Err(BrokerError::InvalidArtifact("invalid content actor".into()));
                }
            }
            if !issuer.starts_with("https://") {
                return Err(BrokerError::InvalidArtifact("invalid content actor".into()));
            }
        }
        Ok(Self { actor, kind })
    }
}

impl crate::FsVaultBroker {
    /// recorded 重放还必须绑定可信 actor/kind，不能把恢复结果重解释为普通保存。
    pub fn mutation_replay_recorded(
        &self,
        request: &crate::Mutation,
        metadata: &MutationMetadata,
    ) -> Result<Option<crate::MutationReceipt>, BrokerError> {
        crate::mutation::validate_request(request)?;
        check_existing(self, request, Some(metadata))?;
        let hash = crate::sha256(request.idempotency_key.as_bytes());
        if store::load_id(self, &store::id(self, &hash))?.is_none()
            && crate::mutation::replay::load(&self.data, &hash, self.writer.binding.pair_id)?
                .is_some()
        {
            return Err(BrokerError::IdempotencyConflict);
        }
        self.mutation_replay(request)
    }
    /// 普通保存允许 Audit 暂不可用，但固定 Intent 必须先可靠落盘。
    pub fn mutate_recorded(
        &mut self,
        request: crate::Mutation,
        metadata: MutationMetadata,
    ) -> Result<crate::MutationReceipt, BrokerError> {
        if matches!(metadata.kind, MutationKind::Restore { .. }) {
            return Err(BrokerError::InvalidArtifact(
                "restore requires audit confirmation".into(),
            ));
        }
        crate::mutation::apply_recorded(self, request, &metadata)
    }
    /// 仅在 CAS 拒绝成为永久终态时产生固定的正文 Intent/Outcome。
    pub fn check_precondition_recorded(
        &mut self,
        request: &crate::Mutation,
        metadata: MutationMetadata,
    ) -> Result<(), BrokerError> {
        self.check_precondition_metadata(request, Some(&metadata))
    }
}
