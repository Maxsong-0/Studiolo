//! MCP 会话 handler：每个 HTTP 会话一个实例；`initialize` 铸实例并把路牌放进 instructions，
//! 四个 tools 全部「先审计后返回」，越白名单一律拒绝并审计。

use std::{
    borrow::Cow,
    future::Future,
    sync::{Arc, OnceLock},
};

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
use studiolo_core::VaultPath;
use studiolo_supply::{
    HARD_BUDGET_TOKENS, Instance, ReadRecord, SearchMode, SearchQuery, SupplyError,
    audit::{self, Actor},
    signpost::{self, Principal, SignpostInput},
};

use crate::{
    auth::AgentType,
    state::{AppState, AuditUnavailable, now},
};

/// 本片只支持带会话的协议版本：实例身份绑定 MCP 会话（SEP-2567 无会话模式另行设计）。
const SESSION_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2025_11_25,
    ProtocolVersion::V_2025_06_18,
    ProtocolVersion::V_2025_03_26,
];

/// tool 清单与一句说明（进入路牌第 6 项）。
pub const TOOLS: &[(&str, &str)] = &[
    ("handshake", "重取冷启动路牌"),
    ("list_entries", "列出一个白名单项目的条目：project"),
    ("read_note", "读一篇笔记：path"),
    (
        "search",
        "全文或路径搜索：project、text、mode=full_text|path",
    ),
];

const MAX_NOTE_BYTES: u64 = 4 * 1024 * 1024;

/// 一个 MCP 会话。
pub struct AgentSession {
    state: Arc<AppState>,
    session_ref: String,
    instance: OnceLock<Instance>,
}

/// `list_entries` 参数。
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ListEntriesArgs {
    /// 白名单项目前缀，例如 `项目/Studiolo`。
    pub project: String,
}

/// `read_note` 参数。
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ReadNoteArgs {
    /// 相对 vault 根的逻辑路径，例如 `项目/Studiolo/00-总览/INDEX.md`。
    pub path: String,
}

/// `search` 参数。
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SearchArgs {
    /// 白名单项目前缀。
    pub project: String,
    /// 查询文本（全文为单行字面串；路径模式为空白分隔片段）。
    pub text: String,
    /// `full_text` 或 `path`。
    #[serde(default)]
    pub mode: Option<String>,
}

fn text_result(text: String) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text)])
}

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
    fn authorize(&self, action: &str) -> Result<Instance, McpError> {
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

    fn deny(&self, actor: Actor<'_>, action: &str, reason: &str) {
        if let Ok(event) = audit::action_denied(actor, action, reason) {
            let _ = self.state.audit(event);
        }
    }

    /// 内容供给前的审计：失败即拒绝返回。
    fn audit_read(
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

    fn allowed_project(
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
    fn signpost(&self, instance: &Instance) -> Result<String, McpError> {
        let generation = self.state.current();
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
                if let Ok(event) =
                    audit::handshake_budget_exceeded(actor, exceeded.tokens, exceeded.limit)
                {
                    let _ = self.state.audit(event);
                }
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

#[tool_router]
impl AgentSession {
    #[tool(
        description = "重取冷启动路牌：我是谁、项目状态、能力、待我做的事、硬约束、继续拉的入口；≤2000 token"
    )]
    async fn handshake(&self) -> Result<CallToolResult, McpError> {
        let instance = self.authorize("handshake")?;
        Ok(text_result(self.signpost(&instance)?))
    }

    #[tool(
        description = "列出一个白名单项目的全部条目（路径、类型、字节数）；每行：kind<TAB>path<TAB>bytes"
    )]
    async fn list_entries(
        &self,
        Parameters(args): Parameters<ListEntriesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.authorize("list_entries")?;
        let prefix = self.allowed_project(&instance, "list_entries", &args.project)?;
        let generation = self.state.current();
        let scope = format!("{prefix}/");
        let mut lines = Vec::new();
        for entry in &generation.snapshot.entries {
            if !entry.path.as_str().starts_with(&scope) {
                continue;
            }
            let kind = match entry.kind {
                studiolo_supply::EntryKind::Directory => "dir",
                studiolo_supply::EntryKind::Markdown => "note",
                studiolo_supply::EntryKind::Attachment => "attachment",
            };
            lines.push(format!(
                "{kind}\t{}\t{}",
                entry.path,
                entry
                    .size_bytes
                    .map_or(String::new(), |bytes| bytes.to_string())
            ));
        }
        self.audit_read(&instance, prefix.as_str(), "-", "list_entries")?;
        Ok(text_result(format!(
            "# 代次 {} · {} 条\n{}",
            generation.number,
            lines.len(),
            lines.join("\n")
        )))
    }

    #[tool(
        description = "读一篇白名单内笔记的当前正文；返回首行 `revision <hex>` 后接正文；越白名单一律拒绝"
    )]
    async fn read_note(
        &self,
        Parameters(args): Parameters<ReadNoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.authorize("read_note")?;
        let actor = Actor::instance(&instance.agent_type, &instance.id);
        let path = match VaultPath::parse(&args.path) {
            Ok(path) => path,
            Err(_) => {
                self.deny(actor, "read_note", "unsafe_path");
                return Err(McpError::invalid_params("路径语法无效", None));
            }
        };
        let note = match self.state.roots.read_note(&path, MAX_NOTE_BYTES) {
            Ok(note) => note,
            Err(SupplyError::OutsideAllowlist) => {
                self.deny(actor, "read_note", "outside_allowlist");
                return Err(McpError::invalid_params("路径不在供给白名单内", None));
            }
            Err(SupplyError::NotFound) => {
                self.deny(actor, "read_note", "not_found");
                return Err(McpError::resource_not_found("路径不存在", None));
            }
            Err(_) => {
                self.deny(actor, "read_note", "unsafe_path");
                return Err(McpError::invalid_params("路径被只读根拒绝", None));
            }
        };
        let revision = note.revision.to_string();
        self.audit_read(&instance, path.as_str(), &revision, "read_note")?;
        Ok(text_result(format!(
            "revision {revision}\n{}",
            note.content
        )))
    }

    #[tool(
        description = "在一个白名单项目内做字面搜索；mode=full_text（默认）或 path；每行：path:line<TAB>snippet；最多 100 条"
    )]
    async fn search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let instance = self.authorize("search")?;
        let prefix = self.allowed_project(&instance, "search", &args.project)?;
        let mode = match args.mode.as_deref() {
            None | Some("full_text") => SearchMode::FullText,
            Some("path") => SearchMode::Path,
            Some(_) => {
                return Err(McpError::invalid_params(
                    "mode 只能是 full_text 或 path",
                    None,
                ));
            }
        };
        let generation = self.state.current();
        let results = generation
            .model
            .search(&SearchQuery {
                text: args.text,
                project: None,
                mode,
            })
            .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        let scope = format!("{prefix}/");
        let hits = results
            .hits
            .into_iter()
            .filter(|hit| hit.path.starts_with(&scope))
            .take(100)
            .collect::<Vec<_>>();
        // 每个命中路径都是一次供给：先审计，任一失败即整体拒绝。
        let mut audited = std::collections::BTreeSet::new();
        for hit in &hits {
            if !audited.insert(hit.path.clone()) {
                continue;
            }
            let revision = VaultPath::parse(&hit.path)
                .ok()
                .and_then(|path| {
                    generation
                        .snapshot
                        .note(&path)
                        .map(|note| note.revision.to_string())
                })
                .unwrap_or_else(|| "-".to_owned());
            self.audit_read(&instance, &hit.path, &revision, "search")?;
        }
        let lines = hits
            .iter()
            .map(|hit| format!("{}:{}\t{}", hit.path, hit.line, hit.snippet))
            .collect::<Vec<_>>();
        Ok(text_result(format!(
            "# 代次 {} · {} 条{}\n{}",
            generation.number,
            lines.len(),
            if results.truncated {
                "（已截断）"
            } else {
                ""
            },
            lines.join("\n")
        )))
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
                Ok(_) => match self.signpost(&instance) {
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
