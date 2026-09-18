//! 策略平面审计事件：动作名与敏感动作枚举表一致，payload 只含类别、subject、版本、哈希、
//! 扩大项计数与结构化原因；不含正文。发布用 Intent／Outcome 同 id 成对写入。

use serde_json::{Value, json};
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};

use crate::{Activation, DesiredRecord, RevocationLevel};

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

fn desired_payload(record: &DesiredRecord) -> Value {
    json!({
        "desired_id": record.id,
        "kind": record.kind.as_str(),
        "subject": record.subject,
        "body_sha256": record.body_sha256,
        "status": record.status.as_str(),
        "ok": record.report.as_ref().is_some_and(|report| report.ok),
        "expansions": record.report.as_ref().map_or(0, |report| report.expansions.len()),
        "base_effective_hash": record.report.as_ref().map(|report| report.base_effective_hash.clone()),
    })
}

/// `policy.desired_submitted`。
pub fn desired_submitted(
    principal: Principal<'_>,
    record: &DesiredRecord,
) -> Result<AuditEvent, AuditError> {
    event(
        fresh_id(DESIRED_SUBMITTED)?,
        AuditPhase::Outcome,
        DESIRED_SUBMITTED,
        principal,
        desired_payload(record),
    )
}

/// `policy.validated`。
pub fn validated(
    principal: Principal<'_>,
    record: &DesiredRecord,
) -> Result<AuditEvent, AuditError> {
    event(
        fresh_id(VALIDATED)?,
        AuditPhase::Outcome,
        VALIDATED,
        principal,
        desired_payload(record),
    )
}

/// 一次发布的稳定审计 id：Intent 与 Outcome 共用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivationId(AuditEventId);

impl ActivationId {
    /// 铸一个新 id。
    pub fn new() -> Result<Self, AuditError> {
        fresh_id(ACTIVATED).map(Self)
    }

    /// 发布前的 Intent：记录要发什么、基于哪个 effective 哈希。
    pub fn intent(
        &self,
        principal: Principal<'_>,
        request: &ActivationRequest<'_>,
    ) -> Result<AuditEvent, AuditError> {
        event(
            self.0.clone(),
            AuditPhase::Intent,
            ACTIVATED,
            principal,
            json!({
                "desired_id": request.desired_id,
                "rollback_of": request.rollback_of,
                "release_global_revocation": request.release_global_revocation,
                "expected_effective_hash": request.expected_effective_hash,
            }),
        )
    }

    /// 发布成功后的 Outcome。
    pub fn outcome(
        &self,
        principal: Principal<'_>,
        activation: &Activation,
    ) -> Result<AuditEvent, AuditError> {
        event(
            self.0.clone(),
            AuditPhase::Outcome,
            ACTIVATED,
            principal,
            json!({
                "decision": "allow",
                "version": activation.version,
                "kind": activation.kind.as_str(),
                "subject": activation.subject,
                "retired": activation.retired,
                "body_sha256": activation.body_sha256,
                "effective_hash": activation.effective_hash,
                "previous_effective_hash": activation.previous_effective_hash,
                "expansions": activation.expansions.len(),
                "desired_id": activation.desired_id,
                "rollback_of": activation.rollback_of,
                "cleared_type_revocation": activation.cleared_type_revocation,
            }),
        )
    }

    /// 解除全局吊销成功后的 Outcome。
    pub fn released_outcome(
        &self,
        principal: Principal<'_>,
        effective_hash: &str,
    ) -> Result<AuditEvent, AuditError> {
        event(
            self.0.clone(),
            AuditPhase::Outcome,
            ACTIVATED,
            principal,
            json!({
                "decision": "allow",
                "release_global_revocation": true,
                "effective_hash": effective_hash,
            }),
        )
    }
}

/// 发布请求的三种形态之一（Intent 只记形态与期望哈希）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivationRequest<'a> {
    /// 要发布的 desired。
    pub desired_id: Option<&'a str>,
    /// 要回滚到的版本。
    pub rollback_of: Option<u64>,
    /// 是否解除全局吊销。
    pub release_global_revocation: bool,
    /// 调用方声明的 effective 哈希。
    pub expected_effective_hash: &'a str,
}

/// `policy.activate_denied`：结构化原因（`reauthentication_required` / `hash_mismatch` /
/// `validation_failed` / `desired_not_found` / …）。
pub fn activate_denied(
    principal: Principal<'_>,
    request: &ActivationRequest<'_>,
    reason: &str,
) -> Result<AuditEvent, AuditError> {
    event(
        fresh_id(ACTIVATE_DENIED)?,
        AuditPhase::Outcome,
        ACTIVATE_DENIED,
        principal,
        json!({
            "decision": "deny",
            "reason": reason,
            "desired_id": request.desired_id,
            "rollback_of": request.rollback_of,
            "release_global_revocation": request.release_global_revocation,
            "expected_effective_hash": request.expected_effective_hash,
        }),
    )
}

/// `policy.bootstrapped`。
pub fn bootstrapped(activation: &Activation) -> Result<AuditEvent, AuditError> {
    event(
        fresh_id(BOOTSTRAPPED)?,
        AuditPhase::Outcome,
        BOOTSTRAPPED,
        Principal::Bootstrap,
        json!({
            "version": activation.version,
            "kind": activation.kind.as_str(),
            "subject": activation.subject,
            "body_sha256": activation.body_sha256,
            "effective_hash": activation.effective_hash,
        }),
    )
}

/// 三级吊销事件（按级别选动作名）。
pub fn revoked(
    principal: Principal<'_>,
    level: RevocationLevel,
    target: &str,
    reason: &str,
) -> Result<AuditEvent, AuditError> {
    let action = match level {
        RevocationLevel::Instance => INSTANCE_REVOKED,
        RevocationLevel::Type => TYPE_REVOKED,
        RevocationLevel::Global => GLOBAL_REVOKED,
    };
    event(
        fresh_id(action)?,
        AuditPhase::Outcome,
        action,
        principal,
        json!({"level": level.as_str(), "target": target, "reason": reason}),
    )
}

/// `agent.global_released`。
pub fn global_released(principal: Principal<'_>) -> Result<AuditEvent, AuditError> {
    event(
        fresh_id(GLOBAL_RELEASED)?,
        AuditPhase::Outcome,
        GLOBAL_RELEASED,
        principal,
        json!({}),
    )
}

/// `agent_type.restored`：类型吊销随能力集重新发布而解除。
pub fn type_restored(
    principal: Principal<'_>,
    ai_type: &str,
    version: u64,
) -> Result<AuditEvent, AuditError> {
    event(
        fresh_id(TYPE_RESTORED)?,
        AuditPhase::Outcome,
        TYPE_RESTORED,
        principal,
        json!({"target": ai_type, "version": version}),
    )
}
