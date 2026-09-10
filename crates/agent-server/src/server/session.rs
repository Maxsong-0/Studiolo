//! 会话认证、先审计后供给及带新鲜度的预算路牌。
use super::{AgentSession, TOOLS};
use crate::state::{AppState, AuditUnavailable, Generation, now, now_ms};
use rmcp::ErrorData as McpError;
use std::sync::{Arc, OnceLock};
use studiolo_core::VaultPath;
use studiolo_supply::{
    HARD_BUDGET_TOKENS, Instance, ReadRecord,
    audit::{self, Actor},
    signpost::{self, Principal, SignpostInput},
};

fn degraded(outage: &AuditUnavailable) -> McpError {
    McpError::internal_error(
        format!(
            "审计不可用（outage {}）：本次读写全部拒绝",
            outage.outage_id
        ),
        None,
    )
}

impl AgentSession {
    /// 新会话：实例在 `initialize` 时铸造。
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            session_ref: ulid::Ulid::generate().to_string(),
            instance: OnceLock::new(),
        }
    }

    /// 每次调用都重验：实例已注册且类型未吊销。
    pub(super) fn authorize(&self, action: &str) -> Result<Instance, McpError> {
        let Some(instance) = self.instance.get() else {
            return Err(McpError::invalid_request("会话尚未完成 initialize", None));
        };
        let active = self
            .state
            .store
            .type_is_active(&instance.agent_type)
            .map_err(|_| McpError::internal_error("身份库不可用", None))?;
        if !active {
            self.deny(
                Actor::instance(&instance.agent_type, &instance.id),
                action,
                "type_revoked",
            );
            return Err(McpError::invalid_request("类型已吊销", None));
        }
        let _ = self.state.store.touch_instance(&instance.id, &now());
        Ok(instance.clone())
    }

    pub(super) fn deny(&self, actor: Actor<'_>, action: &str, reason: &str) {
        if let Ok(event) = audit::action_denied(actor, action, reason) {
            let _ = self.state.audit(event);
        }
    }

    /// 内容供给前的审计：失败即拒绝返回。
    pub(super) fn audit_read(
        &self,
        instance: &Instance,
        path: &str,
        revision: &str,
        tool: &str,
    ) -> Result<(), McpError> {
        let actor = Actor::instance(&instance.agent_type, &instance.id);
        let event = audit::content_read(actor, path, revision, tool)
            .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
        let ack = self
            .state
            .audit(event)
            .map_err(|outage| degraded(&outage))?;
        let _ = self.state.store.record_read(&ReadRecord {
            recorded_at_ms: ack.recorded_at_ms,
            recorded_at: ack.recorded_at,
            agent_type: instance.agent_type.clone(),
            instance_id: instance.id.clone(),
            path: path.to_owned(),
            revision: revision.to_owned(),
            tool: tool.to_owned(),
            ledger_offset: ack.offset,
        });
        Ok(())
    }

    pub(super) fn allowed_project(
        &self,
        instance: &Instance,
        action: &str,
        project: &str,
    ) -> Result<VaultPath, McpError> {
        match self
            .state
            .allow
            .iter()
            .find(|prefix| prefix.as_str() == project)
        {
            Some(prefix) => Ok(prefix.clone()),
            None => {
                self.deny(
                    Actor::instance(&instance.agent_type, &instance.id),
                    action,
                    "outside_allowlist",
                );
                Err(McpError::invalid_params("项目不在供给白名单内", None))
            }
        }
    }

    /// 派生路牌并审计；返回文本。
    pub(super) async fn generation(
        &self,
        instance: &Instance,
        action: &str,
    ) -> Result<Arc<Generation>, McpError> {
        match self.state.verified().await {
            Ok(generation) => Ok(generation),
            Err(reason) => {
                let event = audit::action_denied(
                    Actor::instance(&instance.agent_type, &instance.id),
                    action,
                    "replica_unavailable",
                )
                .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
                self.state
                    .audit(event)
                    .map_err(|outage| degraded(&outage))?;
                Err(McpError::internal_error(
                    format!("副本不可用：{reason}；本次拒绝供给内容"),
                    None,
                ))
            }
        }
    }

    pub(super) fn freshness(generation: &Generation) -> Result<String, McpError> {
        generation
            .freshness
            .as_ref()
            .map_or(Ok(String::new()), |freshness| {
                freshness
                    .describe(now_ms())
                    .map(|text| format!("{text}\n"))
                    .map_err(|error| McpError::internal_error(error.to_string(), None))
            })
    }

    pub(super) async fn signpost(&self, instance: &Instance) -> Result<String, McpError> {
        let generation = self.generation(instance, "handshake").await?;
        let freshness = Self::freshness(&generation)?;
        let principal = Principal {
            agent_type: instance.agent_type.clone(),
            instance_id: instance.id.clone(),
            registered_at: instance.registered_at.clone(),
        };
        let actor = Actor::instance(&instance.agent_type, &instance.id);
        let derived = signpost::derive(&SignpostInput {
            snapshot: &generation.snapshot,
            allow: &self.state.allow,
            principal: &principal,
            generation: generation.number,
            generated_at: &generation.generated_at,
            freshness: (!freshness.is_empty()).then_some(freshness.as_str()),
            tools: TOOLS,
        });
        match derived {
            Ok(signpost) => {
                let event = audit::handshake_served(actor, generation.number, signpost.tokens)
                    .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
                self.state
                    .audit(event)
                    .map_err(|outage| degraded(&outage))?;
                Ok(signpost.text)
            }
            Err(exceeded) => {
                let event =
                    audit::handshake_budget_exceeded(actor, exceeded.tokens, exceeded.limit)
                        .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
                self.state
                    .audit(event)
                    .map_err(|outage| degraded(&outage))?;
                Err(McpError::internal_error(
                    format!(
                        "路牌 {} token 超过硬预算 {}，握手失败（二元验收）",
                        exceeded.tokens, HARD_BUDGET_TOKENS
                    ),
                    None,
                ))
            }
        }
    }
}
