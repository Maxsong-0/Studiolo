//! 黑板权限在转发前与 canonical 提交端各复核一次。
use super::*;
use studiolo_supply::audit::{self, Actor};

#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(crate = "rmcp::schemars")]
pub(super) struct BoardArgs {
    /// 白名单中的完整项目路径。
    pub project: String,
    /// list/get/create/claim/heartbeat/handoff/return/cancel/number。
    pub operation: String,
    /// 严格结构化参数；自然语言请求被拒绝。
    pub request: serde_json::Value,
}

impl AgentSession {
    pub(super) async fn text_board(&self, args: BoardArgs) -> Result<String, McpError> {
        let grant = self.authorize("board")?;
        self.allowed_project(&grant, "board", &args.project)?;
        let policy = grant
            .policy
            .as_ref()
            .ok_or_else(|| McpError::invalid_request("黑板需要有效策略平面", None))?;
        let read = matches!(args.operation.as_str(), "list" | "get");
        if !read
            && (!policy.set.write.board || !policy.set.write.board_projects.contains(&args.project))
        {
            self.deny(grant.actor(), "board", "write_not_allowed");
            return Err(McpError::invalid_request("能力集未授予黑板写权限", None));
        }
        if !matches!(
            args.operation.as_str(),
            "list"
                | "get"
                | "create"
                | "claim"
                | "heartbeat"
                | "handoff"
                | "return"
                | "cancel"
                | "number"
                | "batch.preview"
                | "batch.commit"
                | "batch.rollback"
                | "memory.preview"
        ) || !args.request.is_object()
        {
            return Err(McpError::invalid_params("黑板操作或结构化参数无效", None));
        }
        self.generation(&grant.instance, "board").await?;
        let client = self
            .state
            .board
            .as_ref()
            .ok_or_else(|| McpError::invalid_request("黑板权威服务尚未配置", None))?;
        let event = audit::content_read(
            Actor::instance(&grant.instance.agent_type, &grant.instance.id),
            &args.project,
            &policy.effective_hash,
            "board",
        )
        .map_err(|_| McpError::internal_error("审计构造失败", None))?;
        self.state
            .audit(event)
            .map_err(|_| McpError::internal_error("审计不可用", None))?;
        let request = serde_json::json!({
            "agent_type":grant.instance.agent_type, "instance_id":grant.instance.id,
            "project":args.project, "operation":args.operation, "request":args.request,
        });
        let response = client
            .call(&request)
            .await
            .map_err(|message| McpError::invalid_request(message, None))?;
        // 响应前再次复核，避免在排队与执行期间撤权后仍泄露黑板内容。
        let current = self.authorize("board")?;
        self.allowed_project(&current, "board", &args.project)?;
        if !read
            && current.policy.as_ref().is_none_or(|p| {
                !p.set.write.board || !p.set.write.board_projects.contains(&args.project)
            })
        {
            return Err(McpError::invalid_request(
                "返回前黑板写权限已收回；请刷新权威状态",
                None,
            ));
        }
        serde_json::to_string(&response)
            .map_err(|_| McpError::internal_error("黑板响应编码失败", None))
    }
}

impl AgentSession {
    pub(super) async fn board_signpost(
        &self,
        grant: &super::session::Grant,
    ) -> Result<String, McpError> {
        let Some(client) = &self.state.board else {
            return Ok("黑板实时状态：未接入，A2A 协作暂停。\n".into());
        };
        let projects = grant
            .allowed
            .iter()
            .filter(|p| p.as_str().starts_with("项目/"));
        let mut items = Vec::new();
        for project in projects {
            let response = client
                .call(&serde_json::json!({
                    "agent_type": grant.instance.agent_type, "instance_id": grant.instance.id,
                    "project": project, "operation": "list", "request": {}
                }))
                .await
                .map_err(|_| McpError::internal_error("黑板权威服务不可用，A2A 协作暂停", None))?;
            // During initialize the instance is registered but not yet published into
            // self.instance. Revalidate that explicit identity without accepting a request ID.
            if !self
                .state
                .store
                .type_is_active(&grant.instance.agent_type)
                .map_err(|_| McpError::internal_error("身份库不可用", None))?
                || self.mcp.as_ref().is_some_and(|binding| {
                    binding.stale(std::time::Instant::now()) || !binding.current(&self.state.store)
                })
            {
                return Err(McpError::invalid_request("会话身份已失效", None));
            }
            let current = self.grant_for(&grant.instance, "board")?;
            self.allowed_project(&current, "board", project.as_str())?;
            if current.policy.as_ref().map(|p| &p.effective_hash)
                != grant.policy.as_ref().map(|p| &p.effective_hash)
            {
                return Err(McpError::invalid_request(
                    "读取黑板期间策略变化，请重新握手",
                    None,
                ));
            }
            items.extend(
                response
                    .as_array()
                    .ok_or_else(|| McpError::internal_error("黑板响应无效", None))?
                    .iter()
                    .cloned(),
            );
        }
        let mine = items
            .iter()
            .filter(|v| {
                v.pointer("/board/created_by_instance")
                    .and_then(serde_json::Value::as_str)
                    == Some(grant.instance.id.as_str())
                    && v.pointer("/board/definition/parent")
                        .is_some_and(|p| !p.is_null())
            })
            .collect::<Vec<_>>();
        let active = items
            .iter()
            .filter(|v| {
                v.get("holder").and_then(serde_json::Value::as_str)
                    == Some(grant.instance.id.as_str())
                    && v.pointer("/task/active_attempt")
                        .is_some_and(|v| !v.is_null())
                    && v.pointer("/attempt/lease_until_ms")
                        .and_then(serde_json::Value::as_i64)
                        .is_some_and(|until| until > jiff::Timestamp::now().as_millisecond())
            })
            .count();
        let mut line = format!("黑板实时状态：本实例持有 {active} 项。我派出的子项：");
        for item in &mine {
            let name = item
                .pointer("/task/spec/title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("工作项")
                .chars()
                .take(40)
                .collect::<String>();
            let status = item
                .pointer("/task/status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("未知");
            let files = item
                .pointer("/board/receipt/artifacts")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            let elapsed = item
                .pointer("/board/receipt/elapsed_ms")
                .and_then(serde_json::Value::as_u64)
                .map(|ms| format!("{}秒", ms / 1000))
                .unwrap_or_else(|| "未回执".into());
            line.push_str(&format!("{name}/{status}/{files}文件/{elapsed}；"));
        }
        if mine.is_empty() {
            line.push_str("0 项");
        }
        if studiolo_supply::count_tokens(&line) > 200 {
            line = format!(
                "黑板实时状态：本实例持有 {active} 项；我派出的子项 {} 项，详情用 board list。",
                mine.len()
            );
        }
        line.push('\n');
        Ok(line)
    }
}
