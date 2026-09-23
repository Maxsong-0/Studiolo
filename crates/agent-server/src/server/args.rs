//! 工具参数类型：MCP tools 与 HTTP 续读入口共用，经父模块在原路径 `server::*` 重导出。

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

/// `policy_read` 参数。
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct PolicyReadArgs {
    /// `rules` 或 `skills`。
    pub kind: String,
    /// 不带时列清单；带时读该 subject 的正文（规则母本文件名或 skill 名）。
    #[serde(default)]
    pub subject: Option<String>,
}

/// `propose_desired` 参数。
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ProposeArgs {
    /// `rules`、`skills` 或 `capability_set`。
    pub kind: String,
    /// subject（规则母本文件名／skill 名／AI 类型名）。
    pub subject: String,
    /// 正文（规则为 Markdown，其余为 JSON）。
    pub body: String,
}
