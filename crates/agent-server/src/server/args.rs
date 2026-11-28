//! 工具参数类型：MCP tools 与 HTTP 续读入口共用，经父模块在原路径 `server::*` 重导出。

/// `list_entries` 参数。
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ListEntriesArgs {
    /// 白名单项目前缀，例如 `项目/Studiolo-Docs`。
    pub project: String,
}

/// `read_note` 参数。
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ReadNoteArgs {
    /// 相对 vault 根的逻辑路径，例如 `项目/Studiolo-Docs/00-总览/INDEX.md`。
    pub path: String,
}

/// MCP `read_note` 参数：路径，外加三选一的「只要哪一段」；都不给就是整篇。
/// HTTP 续读入口只按路径读整篇，用 [`ReadNoteArgs`]。
// 三个可选参数收任意 JSON，类型到取到笔记之后才核：反序列化早于授权，在那里拒就没有越界的拒绝和审计。
// 给模型看的声明照样写明类型（`schemars(with)`）。
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ReadNoteToolArgs {
    /// 相对 vault 根的逻辑路径，例如 `项目/Studiolo-Docs/00-总览/INDEX.md`。
    pub path: String,
    /// 只要这几行：`a-b`，从 1 起、两端都含；行号就是 search 和 outline 报的行号。
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub lines: Option<serde_json::Value>,
    /// 只要这个标题的一节（到下一个同级或更高级标题之前）；可以带 `#`，带了就连级别一起核。
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub heading: Option<serde_json::Value>,
    /// 为 true 时只要大纲：每行 `起-止<TAB>## 标题`。
    #[serde(default)]
    #[schemars(with = "Option<bool>")]
    pub outline: Option<serde_json::Value>,
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

/// `list_proposals` 参数；都不带时列本类型全部仍待处理的提案。
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ListProposalsArgs {
    /// 按号查本类型原提案的当前结果（JSON），涵盖已发布、校验失败、撤回、被取代；不能与筛选同用。
    #[serde(default)]
    pub desired_id: Option<String>,
    /// 只列这一类：`rules`、`skills` 或 `capability_set`。
    #[serde(default)]
    pub kind: Option<String>,
    /// 只列这个 subject。
    #[serde(default)]
    pub subject: Option<String>,
}
