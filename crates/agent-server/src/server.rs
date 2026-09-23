//! MCP 会话 handler：每个 HTTP 会话一个实例；`initialize` 铸实例并把路牌放进 instructions，
//! 全部 tools「先审计后返回」，越白名单一律拒绝并审计。
//!
//! M4 起每次调用都复核策略平面：全局／类型／实例三级吊销与该类型的能力集；供给视图 ＝
//! 部署上限 `--allow` ∩ 能力集白名单。`serve` 未给 `--policy` 时退回 M3 兼容模式（只有部署上限）。

mod board;
mod handler;
mod session;

use std::sync::{Arc, OnceLock};

use crate::state::{AppState, SupplySource};
use rmcp::{ErrorData as McpError, model::ProtocolVersion};
use studiolo_core::VaultPath;
use studiolo_policy::{PolicyKind, ProposalInput};
use studiolo_supply::{Instance, SearchMode, SearchQuery, SupplyError};

pub(crate) use session::POLICY_DENIED_PREFIX;

/// 本片只支持带会话的协议版本：实例身份绑定 MCP 会话（SEP-2567 无会话模式另行设计）。
const SESSION_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2025_11_25,
    ProtocolVersion::V_2025_06_18,
    ProtocolVersion::V_2025_03_26,
];

/// tool 清单与一句说明（进入路牌第 6 项）。
pub const TOOLS: &[(&str, &str)] = &[
    ("handshake", "重取冷启动路牌"),
    (
        "board",
        "读写黑板：project、operation、request；写入需有效能力集",
    ),
    ("list_entries", "列出一个白名单项目的条目：project"),
    ("read_note", "读一篇笔记：path"),
    (
        "search",
        "全文或路径搜索：project、text、mode=full_text|path",
    ),
    (
        "policy_read",
        "读策略平面 effective：kind=rules|skills[、subject]",
    ),
    (
        "propose_desired",
        "提交 desired 提案（永不自动生效）：kind、subject、body",
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
    (
        "GET /read/policy?kind=rules|skills[&subject=…]",
        "读策略平面 effective（提案只经 MCP）",
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

impl AgentSession {
    /// 共用只读握手；由外壳选择已计入预算的续读入口。
    pub(crate) async fn text_handshake(&self) -> Result<String, McpError> {
        let grant = self.authorize("handshake")?;
        self.signpost(&grant).await
    }

    /// 共用目录供给；返回前完成该项目读取审计。
    pub(crate) async fn text_list_entries(
        &self,
        args: ListEntriesArgs,
    ) -> Result<String, McpError> {
        let grant = self.authorize("list_entries")?;
        let prefix = self.allowed_project(&grant, "list_entries", &args.project)?;
        let generation = self.generation(&grant.instance, "list_entries").await?;
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
        self.audit_read(&grant.instance, prefix.as_str(), "-", "list_entries")?;
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
        let grant = self.authorize("read_note")?;
        let actor = grant.actor();
        let path = match VaultPath::parse(&args.path) {
            Ok(path) => path,
            Err(_) => {
                self.deny(actor, "read_note", "unsafe_path");
                return Err(McpError::invalid_params("路径语法无效", None));
            }
        };
        // 供给视图先于任何读取：能力集没列的路径从一开始不进入计算。
        if !grant.allows(&path) {
            self.deny(actor, "read_note", "outside_allowlist");
            return Err(McpError::invalid_params("路径不在供给白名单内", None));
        }
        let generation = self.generation(&grant.instance, "read_note").await?;
        let read = match &self.state.source {
            SupplySource::Direct(roots) => roots.read_note(&path, MAX_NOTE_BYTES),
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
        self.audit_read(&grant.instance, path.as_str(), &revision, "read_note")?;
        Ok(format!(
            "revision {revision}\n{}{}",
            Self::freshness(&generation)?,
            note.content
        ))
    }

    /// 共用搜索供给；全部命中审计成功后才返回任何文本。
    pub(crate) async fn text_search(&self, args: SearchArgs) -> Result<String, McpError> {
        let grant = self.authorize("search")?;
        let prefix = self.allowed_project(&grant, "search", &args.project)?;
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
        let generation = self.generation(&grant.instance, "search").await?;
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
            self.audit_read(&grant.instance, &hit.path, &revision, "search")?;
        }
        if hits.is_empty() {
            self.audit_read(&grant.instance, prefix.as_str(), "-", "search")?;
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

    /// 读策略平面 effective：规则母本与 skills 是「读取即外发」的内容，同样先审计后返回。
    pub(crate) async fn text_policy_read(&self, args: PolicyReadArgs) -> Result<String, McpError> {
        let grant = self.authorize("policy_read")?;
        let actor = grant.actor();
        let Some(store) = &self.state.policy else {
            self.deny(actor, "policy_read", "policy_plane_absent");
            return Err(McpError::invalid_request(
                "策略平面未接入（M3 兼容模式）",
                None,
            ));
        };
        let unavailable = |_| McpError::internal_error("策略库不可用，本次拒绝供给", None);
        let kind = match args.kind.as_str() {
            "rules" => PolicyKind::Rules,
            "skills" => PolicyKind::Skills,
            _ => {
                return Err(McpError::invalid_params(
                    "kind 只能是 rules 或 skills",
                    None,
                ));
            }
        };
        // 版本行与条目取自同一读事务：人侧进程在两次读取之间发布，也不会让二者各属一版。
        let header = |hash: &str| format!("# 策略版本 {}\n", &hash[..12]);
        match (kind, args.subject) {
            (PolicyKind::Rules, None) => {
                let rules = store.rules().map_err(unavailable)?;
                self.audit_read(&grant.instance, "policy://rules", "-", "policy_read")?;
                let lines = rules
                    .value
                    .iter()
                    .map(|record| {
                        format!(
                            "{}\tv{}\t{}",
                            record.meta.subject,
                            record.meta.version,
                            &record.meta.body_sha256[..12]
                        )
                    })
                    .collect::<Vec<_>>();
                Ok(format!(
                    "{}{} 份规则母本\n{}",
                    header(&rules.effective_hash),
                    lines.len(),
                    lines.join("\n")
                ))
            }
            (PolicyKind::Rules, Some(subject)) => {
                let rules = store.rules().map_err(unavailable)?;
                let header = header(&rules.effective_hash);
                let Some(record) = rules
                    .value
                    .into_iter()
                    .find(|record| record.meta.subject == subject)
                else {
                    self.deny(actor, "policy_read", "not_found");
                    return Err(McpError::resource_not_found("规则母本不存在", None));
                };
                let path = format!("policy://rules/{subject}");
                let revision = format!("v{}", record.meta.version);
                self.audit_read(&grant.instance, &path, &revision, "policy_read")?;
                Ok(format!(
                    "{header}{path}@{revision} sha256 {}\n{}",
                    record.meta.body_sha256,
                    record.body.unwrap_or_default()
                ))
            }
            (PolicyKind::Skills, None) => {
                let skills = store.skills().map_err(unavailable)?;
                self.audit_read(&grant.instance, "policy://skills", "-", "policy_read")?;
                let lines = skills
                    .value
                    .iter()
                    .map(|(skill, meta)| {
                        format!(
                            "{}\tv{}\t{}\t{}",
                            skill.name,
                            meta.version,
                            skill.summary,
                            skill.triggers.join(" | ")
                        )
                    })
                    .collect::<Vec<_>>();
                Ok(format!(
                    "{}{} 个 skill（索引式候选；硬绑定见路牌第 5 项）\n{}",
                    header(&skills.effective_hash),
                    lines.len(),
                    lines.join("\n")
                ))
            }
            (PolicyKind::Skills, Some(name)) => {
                let skills = store.skills().map_err(unavailable)?;
                let header = header(&skills.effective_hash);
                let Some((skill, meta)) = skills
                    .value
                    .into_iter()
                    .find(|(skill, _)| skill.name == name)
                else {
                    self.deny(actor, "policy_read", "not_found");
                    return Err(McpError::resource_not_found("skill 不存在", None));
                };
                let path = format!("policy://skills/{name}");
                let revision = format!("v{}", meta.version);
                self.audit_read(&grant.instance, &path, &revision, "policy_read")?;
                Ok(format!(
                    "{header}{path}@{revision} sha256 {}\n摘要：{}\n触发：{}\n\n{}",
                    meta.body_sha256,
                    skill.summary,
                    skill.triggers.join(" | "),
                    skill.markdown
                ))
            }
            _ => Err(McpError::invalid_params(
                "kind 只能是 rules 或 skills",
                None,
            )),
        }
    }

    /// 提交 desired 提案：只入库为 `pending`，永不自动生效；须能力集 `policy.propose_desired`。
    pub(crate) async fn text_propose(&self, args: ProposeArgs) -> Result<String, McpError> {
        let grant = self.authorize("propose_desired")?;
        let actor = grant.actor();
        let (Some(store), Some(policy)) = (&self.state.policy, &grant.policy) else {
            self.deny(actor, "propose_desired", "policy_plane_absent");
            return Err(McpError::invalid_request(
                "策略平面未接入（M3 兼容模式）",
                None,
            ));
        };
        if !policy.set.policy.propose_desired {
            self.deny(actor, "propose_desired", "capability_denied");
            return Err(McpError::invalid_request(
                "能力集未授予提交 desired 提案的能力",
                None,
            ));
        }
        let kind = PolicyKind::parse(&args.kind)
            .map_err(|_| McpError::invalid_params("kind 无效", None))?;
        let record = store
            .propose(
                &grant.instance.agent_type,
                &grant.instance.id,
                ProposalInput {
                    kind,
                    subject: args.subject,
                    body: args.body,
                },
                &crate::state::now(),
            )
            .map_err(|error| match error {
                studiolo_policy::PolicyError::TooManyProposals => {
                    McpError::invalid_request("待处理提案已达上限，请等待主人处理", None)
                }
                studiolo_policy::PolicyError::InvalidSubject
                | studiolo_policy::PolicyError::InvalidKind
                | studiolo_policy::PolicyError::InvalidBody(_) => {
                    McpError::invalid_params(error.to_string(), None)
                }
                _ => McpError::internal_error("策略库不可用，本次拒绝", None),
            })?;
        let event = studiolo_policy::audit::desired_submitted(
            studiolo_policy::audit::Principal::Agent {
                agent_type: &grant.instance.agent_type,
                instance_id: &grant.instance.id,
            },
            &record,
        )
        .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
        self.state
            .audit(event)
            .map_err(|outage| session::degraded(&outage))?;
        let report = record.report.as_ref();
        let problems = report
            .map(|report| {
                report
                    .problems
                    .iter()
                    .map(|problem| format!("- {}：{}", problem.path, problem.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        Ok(format!(
            "desired_id {}\nstatus {}\nok {}\nexpansions {}\n提案永不自动生效；须主人在人侧经 One-time Code 发布。\n{problems}",
            record.id,
            record.status.as_str(),
            report.is_some_and(|report| report.ok),
            report.map_or(0, |report| report.expansions.len()),
        ))
    }
}
