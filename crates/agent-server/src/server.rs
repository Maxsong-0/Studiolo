//! MCP 会话 handler：每个 HTTP 会话一个实例；`initialize` 铸实例并把路牌放进 instructions，
//! 四个 tools 全部「先审计后返回」，越白名单一律拒绝并审计。

mod handler;
mod session;

use std::sync::{Arc, OnceLock};

use crate::state::{AppState, SupplySource};
use rmcp::{ErrorData as McpError, model::ProtocolVersion};
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

/// HTTP 续读说明放在入口名内，使各裁剪等级均保留并纳入预算。
const HTTP_TOOLS: &[(&str, &str)] = &[
    (
        "所有 GET 均带 Authorization: Bearer <类型令牌>；续读带 Studiolo-Session-Id: <握手响应头值>",
        "会话过期后重新握手",
    ),
    ("GET /read/handshake", "首次握手或重取冷启动路牌"),
    ("GET /read/entries?project=…", "列出白名单项目条目"),
    ("GET /read/note?path=…", "读取白名单笔记"),
    (
        "GET /read/search?project=…&text=…&mode=full_text|path",
        "查询参数需 URL 编码",
    ),
];

const MAX_NOTE_BYTES: u64 = 4 * 1024 * 1024;

/// 一个 MCP 会话。
pub struct AgentSession {
    state: Arc<AppState>,
    session_ref: String,
    instance: OnceLock<Instance>,
    http: bool,
    mcp: Option<Arc<crate::mcp_transport::Binding>>,
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

impl AgentSession {
    /// 共用只读握手；由外壳选择已计入预算的续读入口。
    pub(crate) async fn text_handshake(&self) -> Result<String, McpError> {
        let instance = self.authorize("handshake")?;
        self.signpost(&instance).await
    }

    /// 共用目录供给；返回前完成该项目读取审计。
    pub(crate) async fn text_list_entries(
        &self,
        args: ListEntriesArgs,
    ) -> Result<String, McpError> {
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
        Ok(format!(
            "{}# 代次 {} · {} 条\n{}",
            Self::freshness(&generation)?,
            generation.number,
            lines.len(),
            lines.join("\n")
        ))
    }

    /// 共用笔记供给；白名单、版本和审计不依赖传输外壳。
    pub(crate) async fn text_read_note(&self, args: ReadNoteArgs) -> Result<String, McpError> {
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
        Ok(format!(
            "revision {revision}\n{}{}",
            Self::freshness(&generation)?,
            note.content
        ))
    }

    /// 共用搜索供给；全部命中审计成功后才返回任何文本。
    pub(crate) async fn text_search(&self, args: SearchArgs) -> Result<String, McpError> {
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
        Ok(format!(
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
        ))
    }
}
