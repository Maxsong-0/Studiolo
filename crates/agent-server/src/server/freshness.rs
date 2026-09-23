//! 新鲜度与代次：取已校验的供给代次（副本不可用即审计后拒绝），并渲染副本新鲜度行。
use super::{AgentSession, auditing::degraded};
use crate::state::{Generation, now_ms};
use rmcp::ErrorData as McpError;
use std::sync::Arc;
use studiolo_supply::{
    Instance,
    audit::{self, Actor},
};

impl AgentSession {
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
}
