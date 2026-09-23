//! 会话认证、策略平面复核（三级吊销与能力集）、先审计后供给，及带新鲜度与策略版本的预算路牌。
//! 本文件留会话构造、MCP 绑定与 initialize；授权复核、审计辅助、新鲜度与代次、路牌分别在同级
//! grant／auditing／freshness／signpost 模块。
use super::AgentSession;
use crate::state::{AppState, now};
use rmcp::ErrorData as McpError;
use std::sync::{Arc, OnceLock};
use studiolo_supply::{
    Instance,
    audit::{self, Actor},
};

/// 策略平面拒绝（吊销／零权限）的固定前缀：握手时像降级握手一样「应答但一项不给」，
/// 外壳据此把它当授权拒绝而不是会话失效。
pub(crate) const POLICY_DENIED_PREFIX: &str = "策略拒绝：";

impl AgentSession {
    /// 新会话：实例在 `initialize` 时铸造。
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            session_ref: ulid::Ulid::generate().to_string(),
            instance: OnceLock::new(),
            http: false,
            mcp: None,
        }
    }

    /// MCP 外壳在预占后构造，绑定实际传输会话的身份与期限。
    pub(crate) fn new_mcp(
        state: Arc<AppState>,
        binding: Arc<crate::mcp_transport::Binding>,
    ) -> Self {
        Self {
            mcp: Some(binding),
            ..Self::new(state)
        }
    }

    pub(super) fn finish_mcp(&self, text: String) -> Result<String, McpError> {
        if let Some(binding) = &self.mcp
            && (!binding.current(&self.state.store) || !binding.touch(std::time::Instant::now()))
        {
            return Err(McpError::invalid_request("会话无效，请重新握手", None));
        }
        Ok(text)
    }

    /// HTTP 会话共用供给逻辑，只替换预算内的续读入口。
    pub(crate) fn new_http(state: Arc<AppState>) -> Self {
        Self {
            http: true,
            ..Self::new(state)
        }
    }

    /// 会话登记后绑定的类型，供 HTTP 外壳匹配每次请求的认证身份。
    pub(crate) fn bound_type(&self) -> Option<&str> {
        self.instance
            .get()
            .map(|instance| instance.agent_type.as_str())
    }

    /// 已登记的实例；外壳可回传身份头，不包含会话句柄或类型令牌。
    pub(crate) fn bound_instance(&self) -> Option<&Instance> {
        self.instance.get()
    }

    /// 由外壳在新会话发布前调用一次；注册、策略复核和路牌均先审计后交付。
    pub(crate) async fn initialize_text(&self, agent_type: &str) -> Result<String, McpError> {
        self.initialize_with_parent(agent_type, None).await
    }

    pub(crate) async fn initialize_with_parent(
        &self,
        agent_type: &str,
        parent: Option<&str>,
    ) -> Result<String, McpError> {
        let instance = if let Some(parent) = parent {
            if self.state.policy.is_none() {
                return Err(McpError::invalid_request("子实例需要有效策略平面", None));
            }
            let ancestors = self
                .state
                .store
                .instance_lineage(parent)
                .map_err(|_| McpError::invalid_params("父实例无效", None))?;
            let parent_instance = ancestors
                .first()
                .filter(|instance| instance.agent_type == agent_type)
                .ok_or_else(|| McpError::invalid_params("父实例类型不匹配", None))?;
            self.grant_for(parent_instance, "instance.register_child")?;
            self.state
                .store
                .register_child_instance(agent_type, parent, &self.session_ref, &now())
        } else {
            self.state
                .store
                .register_instance(agent_type, &self.session_ref, &now())
        }
        .map_err(|_| McpError::internal_error("实例注册失败", None))?;
        let actor = Actor::instance(&instance.agent_type, &instance.id);
        let registered = audit::instance_registered(actor)
            .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
        let instructions = match self.state.audit(registered) {
            Err(outage) => Err(McpError::internal_error(
                format!(
                    "审计不可用：outage {}。本次会话所有读取与写入全部拒绝，不提供路牌；请稍后重试。",
                    outage.outage_id
                ),
                None,
            )),
            Ok(_) => match self.grant_for(&instance, "handshake") {
                Ok(grant) => self.signpost(&grant).await,
                Err(error) => Err(error),
            },
        };
        // MCP 仍保留降级实例与被策略拒绝的实例，使初始化返回原有无内容 instructions
        // （后续每次调用都会再次复核并再次拒绝）；HTTP 分别映射为 503 与 403。
        if instructions.is_ok()
            || instructions.as_ref().is_err_and(|error| {
                error.message.contains("审计不可用")
                    || error.message.starts_with(POLICY_DENIED_PREFIX)
            })
        {
            let _ = self.instance.set(instance);
        }
        instructions
    }
}
