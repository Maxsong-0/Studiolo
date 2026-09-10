//! MCP initialize：实例登记与初次只读路牌。
use super::{AgentSession, SESSION_VERSIONS};
use crate::{auth::AgentType, state::now};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    model::{
        Implementation, InitializeRequestParams, InitializeResult, ProtocolVersion,
        ServerCapabilities, ServerInfo,
    },
    service::{RequestContext, RoleServer},
    tool_handler,
};
use std::{borrow::Cow, future::Future};
use studiolo_supply::audit::{self, Actor};

#[tool_handler]
impl ServerHandler for AgentSession {
    fn get_info(&self) -> ServerInfo {
        let mut server_info = Implementation::from_build_env();
        server_info.name = "studiolo-agent".into();
        let mut info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build());
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        info.server_info = server_info;
        info
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SESSION_VERSIONS)
    }

    fn initialize(
        &self,
        _request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<InitializeResult, McpError>> + Send + '_ {
        let agent_type = context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<AgentType>())
            .map(|agent_type| agent_type.0.clone());
        async move {
            let Some(agent_type) = agent_type else {
                // 门禁中间件缺失时失败关闭，不给任何路牌。
                return Err(McpError::invalid_request("未认证", None));
            };
            let instance = self
                .state
                .store
                .register_instance(&agent_type, &self.session_ref, &now())
                .map_err(|_| McpError::internal_error("实例注册失败", None))?;
            let actor = Actor::instance(&instance.agent_type, &instance.id);
            let registered = audit::instance_registered(actor)
                .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
            let instructions = match self.state.audit(registered) {
                Err(outage) => format!(
                    "审计不可用：outage {}。本次会话所有读取与写入全部拒绝，不提供路牌；请稍后重试。",
                    outage.outage_id
                ),
                Ok(_) => match self.signpost(&instance).await {
                    Ok(text) => text,
                    Err(error) if error.message.contains("审计不可用") => {
                        error.message.to_string()
                    }
                    Err(error) => return Err(error),
                },
            };
            let _ = self.instance.set(instance);
            Ok(self.get_info().with_instructions(instructions))
        }
    }
}
