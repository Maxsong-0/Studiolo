//! 发布事件族：同 id 成对的 Intent／Outcome、发布被拒与首版 bootstrap。

use serde_json::json;
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditEventId, AuditPhase};

use crate::Activation;

use super::{ACTIVATE_DENIED, ACTIVATED, BOOTSTRAPPED, Principal, event, fresh_id};

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
