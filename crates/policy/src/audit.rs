//! 策略平面审计事件：动作名与敏感动作枚举表一致，payload 只含类别、subject、版本、哈希、
//! 扩大项计数与结构化原因；不含正文。发布用 Intent／Outcome 同 id 成对写入。

mod activation;
mod desired;
mod revocation;

use serde_json::Value;
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};

pub use activation::{ActivationId, ActivationRequest, activate_denied, bootstrapped};
pub use desired::{desired_submitted, proposal_submitted, validated};
pub use revocation::{global_released, revoked, type_restored};

/// desired 提交（主人或接入方）。
pub const DESIRED_SUBMITTED: &str = "policy.desired_submitted";
/// desired 复核。
pub const VALIDATED: &str = "policy.validated";
/// 发布（含回滚与解除全局吊销）；Intent 在提交前、Outcome 在提交后。
pub const ACTIVATED: &str = "policy.activated";
/// 发布被拒（L3 缺失、哈希不匹配、校验失败）。
pub const ACTIVATE_DENIED: &str = "policy.activate_denied";
/// 可信本地 CLI 首版 bootstrap。
pub const BOOTSTRAPPED: &str = "policy.bootstrapped";
/// 实例吊销。
pub const INSTANCE_REVOKED: &str = "agent_instance.revoked";
/// 类型吊销。
pub const TYPE_REVOKED: &str = "agent_type.revoked";
/// 全局吊销。
pub const GLOBAL_REVOKED: &str = "agent.global_revoked";
/// 全局吊销解除（L3）。
pub const GLOBAL_RELEASED: &str = "agent.global_released";
/// 类型吊销随能力集重新发布而解除。
pub const TYPE_RESTORED: &str = "agent_type.restored";

/// 事件主体。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Principal<'a> {
    /// 主人（人侧 L2／L3 会话）。
    Owner,
    /// 接入方实例。
    Agent {
        /// 类型名。
        agent_type: &'a str,
        /// 实例 id。
        instance_id: &'a str,
    },
    /// 可信本地 CLI。
    Bootstrap,
}

fn base(principal: Principal<'_>) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    let (actor, agent_type, instance) = match principal {
        Principal::Owner => ("owner", None, None),
        Principal::Agent {
            agent_type,
            instance_id,
        } => ("agent", Some(agent_type), Some(instance_id)),
        Principal::Bootstrap => ("bootstrap", None, None),
    };
    map.insert("actor".into(), actor.into());
    map.insert(
        "agent_type".into(),
        agent_type.map_or(Value::Null, Into::into),
    );
    map.insert(
        "agent_instance".into(),
        instance.map_or(Value::Null, Into::into),
    );
    map
}

fn event(
    id: AuditEventId,
    phase: AuditPhase,
    action: &str,
    principal: Principal<'_>,
    extra: Value,
) -> Result<AuditEvent, AuditError> {
    let mut payload = base(principal);
    if let Value::Object(map) = extra {
        payload.extend(map);
    }
    let payload = SanitizedPayload::try_from(Value::Object(payload))?;
    Ok(AuditEvent::new(id, phase, action, payload))
}

fn fresh_id(action: &str) -> Result<AuditEventId, AuditError> {
    AuditEventId::parse(format!("policy.v1:{action}:{}", ulid::Ulid::generate()))
}
