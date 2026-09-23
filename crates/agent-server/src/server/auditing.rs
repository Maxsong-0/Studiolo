//! 审计辅助：拒绝事件、内容读取审计（附读取索引），以及审计不可用时统一的拒绝错误。
use super::AgentSession;
use crate::state::AuditUnavailable;
use rmcp::ErrorData as McpError;
use studiolo_supply::{
    Instance, ReadRecord,
    audit::{self, Actor},
};

pub(super) fn degraded(outage: &AuditUnavailable) -> McpError {
    McpError::internal_error(
        format!(
            "审计不可用（outage {}）：本次读写全部拒绝",
            outage.outage_id
        ),
        None,
    )
}

impl AgentSession {
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
}
