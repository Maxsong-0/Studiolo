//! 内容读取工具族：握手、条目清单、笔记与搜索；每次先授权复核，所有供给先审计后返回。

use super::{AgentSession, ListEntriesArgs, ReadNoteArgs, SearchArgs};
use crate::state::SupplySource;
use rmcp::ErrorData as McpError;
use studiolo_core::VaultPath;
use studiolo_supply::{SearchMode, SearchQuery, SupplyError};

const MAX_NOTE_BYTES: u64 = 4 * 1024 * 1024;

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
            // 名单外的规则文件与白名单外一样不进入清单，计数随之一致（ADR-026 D5）。
            if !entry.path.as_str().starts_with(&scope)
                || !grant.allows_rule_file(entry.path.as_str())
            {
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
        // 供给视图先于任何读取：能力集没列的路径从一开始不进入计算。先判前缀；前缀内的规则文件
        // 不在规则名单时审计记真实原因，对外与白名单外同一句，不暴露该规则是否存在（ADR-026 D5）。
        if !grant.allows(&path) {
            let reason = if grant.in_view(&path) {
                "rule_not_whitelisted"
            } else {
                "outside_allowlist"
            };
            self.deny(actor, "read_note", reason);
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
        // 名单外的规则文件先滤掉再截前 100 条：既不命中，也不占命中名额。
        let hits = results
            .hits
            .into_iter()
            .filter(|hit| hit.path.starts_with(&scope) && grant.allows_rule_file(&hit.path))
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
}
