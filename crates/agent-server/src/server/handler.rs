//! MCP initialize：实例登记与初次只读路牌。
use super::{AgentSession, ListEntriesArgs, ReadNoteArgs, SESSION_VERSIONS, SearchArgs};
use crate::auth::AgentType;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, ContentBlock, Implementation, InitializeRequestParams, InitializeResult,
        ProtocolVersion, ServerCapabilities, ServerInfo,
    },
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
};
use std::{borrow::Cow, future::Future};

fn text_result(text: String) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text)])
}

#[tool_router]
impl AgentSession {
    fn enter_mcp_work(&self) -> Result<Option<tokio::sync::OwnedSemaphorePermit>, McpError> {
        self.mcp
            .as_ref()
            .map(|binding| {
                binding
                    .enter_work()
                    .ok_or_else(|| McpError::internal_error("供给请求名额已满，请稍后重试", None))
            })
            .transpose()
    }

    #[tool(
        description = "重取冷启动路牌：我是谁、项目状态、能力、待我做的事、硬约束、继续拉的入口；≤2000 token"
    )]
    async fn handshake(&self) -> Result<CallToolResult, McpError> {
        let _work = self.enter_mcp_work()?;
        Ok(text_result(self.finish_mcp(self.text_handshake().await?)?))
    }

    #[tool(
        description = "列出一个白名单项目的全部条目（路径、类型、字节数）；每行：kind<TAB>path<TAB>bytes"
    )]
    async fn list_entries(
        &self,
        Parameters(args): Parameters<ListEntriesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let _work = self.enter_mcp_work()?;
        Ok(text_result(
            self.finish_mcp(self.text_list_entries(args).await?)?,
        ))
    }

    #[tool(
        description = "读一篇白名单内笔记的当前正文；返回首行 `revision <hex>` 后接正文；越白名单一律拒绝"
    )]
    async fn read_note(
        &self,
        Parameters(args): Parameters<ReadNoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        let _work = self.enter_mcp_work()?;
        Ok(text_result(
            self.finish_mcp(self.text_read_note(args).await?)?,
        ))
    }

    #[tool(
        description = "在一个白名单项目内做字面搜索；mode=full_text（默认）或 path；每行：path:line<TAB>snippet；最多 100 条"
    )]
    async fn search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let _work = self.enter_mcp_work()?;
        Ok(text_result(self.finish_mcp(self.text_search(args).await?)?))
    }
}

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
            let _work = self.enter_mcp_work()?;
            let Some(agent_type) = agent_type else {
                // 门禁中间件缺失时失败关闭，不给任何路牌。
                return Err(McpError::invalid_request("未认证", None));
            };
            if self
                .mcp
                .as_ref()
                .is_some_and(|binding| binding.owner != agent_type)
            {
                return Err(McpError::invalid_request("会话身份不匹配", None));
            }
            let instructions = match self.initialize_text(&agent_type).await {
                Ok(text) => text,
                Err(error) if error.message.contains("审计不可用") => {
                    error.message.to_string()
                }
                Err(error) => return Err(error),
            };
            Ok(self.get_info().with_instructions(instructions))
        }
    }
}
