//! MCP 会话 handler：每个 HTTP 会话一个实例；`initialize` 铸实例并把路牌放进 instructions，
//! 四个 tools 全部「先审计后返回」，越白名单一律拒绝并审计。

mod handler;
mod session;

use std::sync::{Arc, OnceLock};

use crate::state::{AppState, SupplySource};
use rmcp::{
    ErrorData as McpError,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ProtocolVersion},
    tool, tool_router,
};
use studiolo_core::VaultPath;
use studiolo_supply::{Instance, SearchMode, SearchQuery, SupplyError, audit::Actor};

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

#[tool_router]
impl AgentSession {
    #[tool(
        description = "重取冷启动路牌：我是谁、项目状态、能力、待我做的事、硬约束、继续拉的入口；≤2000 token"
    )]
    async fn handshake(&self) -> Result<CallToolResult, McpError> {
        let instance = self.authorize("handshake")?;
        Ok(text_result(self.signpost(&instance).await?))
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
        let generation = self.generation(&instance, "list_entries").await?;
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
            "{}# 代次 {} · {} 条\n{}",
            Self::freshness(&generation)?,
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
        let generation = self.generation(&instance, "read_note").await?;
        let read = match &self.state.source {
            SupplySource::Direct(roots) => roots.read_note(&path, MAX_NOTE_BYTES),
            SupplySource::Replica(_)
                if !self
                    .state
                    .allow
                    .iter()
                    .any(|prefix| path.as_str().starts_with(&format!("{prefix}/"))) =>
            {
                Err(SupplyError::OutsideAllowlist)
            }
            SupplySource::Replica(_) => generation
                .snapshot
                .note(&path)
                .cloned()
                .ok_or(SupplyError::NotFound),
        };
        let note = match read {
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
            "revision {revision}\n{}{}",
            Self::freshness(&generation)?,
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
        let generation = self.generation(&instance, "search").await?;
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
        if hits.is_empty() {
            self.audit_read(&instance, prefix.as_str(), "-", "search")?;
        }
        let lines = hits
            .iter()
            .map(|hit| format!("{}:{}\t{}", hit.path, hit.line, hit.snippet))
            .collect::<Vec<_>>();
        Ok(text_result(format!(
            "{}# 代次 {} · {} 条{}\n{}",
            Self::freshness(&generation)?,
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
