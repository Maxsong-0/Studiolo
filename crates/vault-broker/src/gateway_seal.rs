//! Gateway 每次调用的私有不可变封存；不占最终 attempt artifacts 的 manifest。
mod store;
#[cfg(test)]
mod tests;
use crate::{AttemptId, BrokerError, Fence, TaskId, VaultPath};
use serde::{Deserialize, Serialize};

/// 可信 Gateway 提交的元数据；无 raw 正文或环境路径。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayCallSeal {
    /// 所属任务。
    pub task_id: TaskId,
    /// 所属 attempt。
    pub attempt_id: AttemptId,
    /// 当前已验证的 fence。
    pub fence: Fence,
    /// 稳定逻辑调用 ID；Broker 对其哈希后才用于路径。
    pub call_id: String,
    /// 已持久化的审计 intent 关联。
    pub audit_intent: String,
    /// Gateway 私有 raw 对象的完整摘要。
    pub raw_sha256: String,
    /// 私有 raw 对象的完整字节数。
    pub raw_bytes: u64,
    /// 全部源信息清单的摘要。
    pub source_manifest_sha256: String,
    /// 固定模型供应商。
    pub model_provider: String,
    /// 固定模型名。
    pub model: String,
    /// 不得超过源保留期限的绝对 UTC Unix 秒。
    pub retain_until_unix_seconds: i64,
    /// Broker 输入快照的内容寻址 ID。
    pub snapshot_id: String,
    /// 已生效策略的摘要。
    pub effective_policy_hash: String,
}
/// 元数据 manifest 原子持久化后的回执；不表明 raw 在别处的保留状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayCallReceipt {
    /// 固定的私有 Vault 逻辑 manifest 路径。
    pub manifest_path: VaultPath,
    /// 完整 manifest 字节的 SHA-256。
    pub sha256: String,
}
/// 每次 Gateway 调用的独立发布能力；主体/预算/fence 准入由调用方完成。
pub trait GatewayPublisher: Send + Sync {
    /// 同 call 同元数据重放原回执，同 call 不同元数据拒绝；不复制 raw。
    fn seal_gateway_call(
        &mut self,
        seal: GatewayCallSeal,
    ) -> Result<GatewayCallReceipt, BrokerError>;
}
