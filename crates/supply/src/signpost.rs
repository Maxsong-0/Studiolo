//! 冷启动路牌：六项全部从白名单内的结构化 Markdown 派生，不读任何人写的「路牌文件」。
//!
//! 只给路牌不给正文（ADR-015 二）。超过内部目标时按固定优先级逐级裁剪，最后由硬预算兜底：
//! 超出 2000 token 即失败，不是告警。

use std::fmt::Write as _;

use studiolo_core::VaultPath;

use crate::{BudgetExceeded, HARD_BUDGET_TOKENS, Snapshot, TARGET_BUDGET_TOKENS, check_budget};

/// 接入方身份（类型 ＋ 实例）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    /// 类型名（claude / codex / grok …）。
    pub agent_type: String,
    /// 实例 id（ULID）。
    pub instance_id: String,
    /// 实例注册时间（RFC 3339）。
    pub registered_at: String,
}

/// 路牌派生输入。
pub struct SignpostInput<'a> {
    /// 当前快照。
    pub snapshot: &'a Snapshot,
    /// 白名单前缀（配置顺序）。
    pub allow: &'a [VaultPath],
    /// 接入方身份。
    pub principal: &'a Principal,
    /// 快照代次。
    pub generation: u64,
    /// 生成时间（RFC 3339）。
    pub generated_at: &'a str,
    /// 本外壳暴露的 tool 名与一句说明。
    pub tools: &'a [(&'a str, &'a str)],
}

/// 派生结果：文本、计量与用于审计的摘要。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signpost {
    /// 路牌全文（Markdown）。
    pub text: String,
    /// o200k 计量。
    pub tokens: usize,
    /// 生效的裁剪等级（0 = 未裁剪）。
    pub trim_level: u8,
}

/// 一个白名单项目的派生状态；缺失字段显示「不可用」。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectStatus {
    prefix: String,
    fields: Vec<(String, String)>,
    focus: Option<String>,
    in_progress: Vec<String>,
    unclaimed: Vec<String>,
    entries: Vec<(String, String)>,
}

const STATUS_KEYS: [&str; 4] = ["当前版本", "状态", "最近日志", "未关闭 BUG"];

/// 派生并计量路牌；超过硬预算返回错误（二元验收）。
pub fn derive(input: &SignpostInput<'_>) -> Result<Signpost, BudgetExceeded> {
    let projects = input
        .allow
        .iter()
        .filter_map(|prefix| project_status(input.snapshot, prefix))
        .collect::<Vec<_>>();
    let rules = rules_summary(input.snapshot, input.allow);
    let mut last = None;
    for level in 0..=3u8 {
        let text = render(input, &projects, &rules, level);
        match check_budget(&text) {
            Ok(tokens) if tokens <= TARGET_BUDGET_TOKENS => {
                return Ok(Signpost {
                    text,
                    tokens,
                    trim_level: level,
                });
            }
            Ok(tokens) => last = Some((text, tokens, level)),
            Err(error) => {
                if level == 3 {
                    return Err(error);
                }
            }
        }
    }
    // 三级裁剪后仍在目标之上但未超硬预算：接受，并记录裁剪等级。
    let (text, tokens, level) = last.ok_or(BudgetExceeded {
        tokens: usize::MAX,
        limit: HARD_BUDGET_TOKENS,
    })?;
    Ok(Signpost {
        text,
        tokens,
        trim_level: level,
    })
}

fn render(
    input: &SignpostInput<'_>,
    projects: &[ProjectStatus],
    rules: &RulesSummary,
    level: u8,
) -> String {
    let focus_chars = [200usize, 140, 100, 60][usize::from(level)];
    let task_chars = [80usize, 60, 40, 30][usize::from(level)];
    let max_tasks = [8usize, 5, 3, 2][usize::from(level)];
    let max_unclaimed = [3usize, 2, 1, 0][usize::from(level)];
    let field_chars = [120usize, 90, 60, 40][usize::from(level)];
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Studiolo 路牌（代次 {}，{}）",
        input.generation, input.generated_at
    );
    let _ = writeln!(
        out,
        "## 1 我是谁\n类型 {} · 实例 {} · 注册 {}",
        input.principal.agent_type, input.principal.instance_id, input.principal.registered_at
    );
    out.push_str("## 2 项目状态\n");
    if projects.is_empty() {
        out.push_str("白名单内没有带 00-总览/INDEX.md 的项目。\n");
    }
    for project in projects {
        let _ = writeln!(out, "### {}", project.prefix);
        for key in STATUS_KEYS {
            let value = project
                .fields
                .iter()
                .find(|(name, _)| name == key)
                .map_or("不可用", |(_, value)| value.as_str());
            let _ = writeln!(out, "{key}：{}", truncate(value, field_chars));
        }
        if level < 3 {
            let _ = writeln!(
                out,
                "当前重点：{}",
                project
                    .focus
                    .as_deref()
                    .map_or_else(|| "不可用".to_owned(), |focus| truncate(focus, focus_chars))
            );
        }
    }
    let _ = writeln!(
        out,
        "## 3 能力\n只读；白名单：{}；写与审批：无（M3）",
        input
            .allow
            .iter()
            .map(|prefix| prefix.as_str())
            .collect::<Vec<_>>()
            .join("、")
    );
    out.push_str("## 4 待我做的事\n");
    let mut any = false;
    for project in projects {
        for row in project.in_progress.iter().take(max_tasks) {
            let _ = writeln!(out, "进行中：{}", truncate(row, task_chars + 40));
            any = true;
        }
        for title in project.unclaimed.iter().take(max_unclaimed) {
            let _ = writeln!(out, "待认领：{}", truncate(title, task_chars));
            any = true;
        }
    }
    if !any {
        out.push_str("无（协调板无进行中或待认领项，或不可用）\n");
    }
    out.push_str("## 5 硬约束\n");
    if rules.principles.is_empty() {
        out.push_str("不可用（白名单内无规则母本）\n");
    } else {
        let _ = writeln!(
            out,
            "{}",
            rules
                .principles
                .iter()
                .enumerate()
                .map(|(index, title)| format!("{} {}", index + 1, truncate(title, 24)))
                .collect::<Vec<_>>()
                .join("；")
        );
    }
    for binding in rules
        .hard_bindings
        .iter()
        .take(if level >= 2 { 1 } else { 3 })
    {
        let _ = writeln!(
            out,
            "硬绑定：{}",
            truncate(binding, if level >= 2 { 80 } else { 160 })
        );
    }
    out.push_str("## 6 继续拉\n");
    let _ = writeln!(
        out,
        "tools：{}",
        input
            .tools
            .iter()
            .map(|(name, description)| if level >= 1 {
                (*name).to_owned()
            } else {
                format!("{name}（{description}）")
            })
            .collect::<Vec<_>>()
            .join("、")
    );
    for project in projects {
        for (path, revision) in project.entries.iter().take(if level >= 2 { 1 } else { 3 }) {
            let _ = writeln!(out, "入口：{path}@{}", &revision[..12.min(revision.len())]);
        }
    }
    for (path, revision) in &rules.entries {
        let _ = writeln!(out, "入口：{path}@{}", &revision[..12.min(revision.len())]);
    }
    if !input.snapshot.diagnostics.is_empty() {
        let _ = writeln!(
            out,
            "诊断：{} 条扫描诊断，视图可能不完整",
            input.snapshot.diagnostics.len()
        );
    }
    out
}

fn project_status(snapshot: &Snapshot, prefix: &VaultPath) -> Option<ProjectStatus> {
    let index_path = format!("{prefix}/00-总览/INDEX.md");
    let index = snapshot
        .notes
        .iter()
        .find(|note| note.path.as_str() == index_path)?;
    let mut status = ProjectStatus {
        prefix: prefix.as_str().to_owned(),
        ..ProjectStatus::default()
    };
    status.fields = first_table_fields(&index.content);
    status.focus = section_first_bullet(&index.content, "## 当前重点");
    status
        .entries
        .push((index_path.clone(), index.revision.to_string()));
    let tasks_path = format!("{prefix}/04-协调/TASKS.md");
    if let Some(tasks) = snapshot
        .notes
        .iter()
        .find(|note| note.path.as_str() == tasks_path)
    {
        status.in_progress = in_progress_rows(&tasks.content);
        status.unclaimed = unclaimed_titles(&tasks.content);
        status
            .entries
            .push((tasks_path, tasks.revision.to_string()));
    }
    let log_prefix = format!("{prefix}/02-日志/");
    if let Some(latest) = snapshot
        .notes
        .iter()
        .filter(|note| note.path.as_str().starts_with(&log_prefix))
        .max_by(|left, right| left.path.as_str().cmp(right.path.as_str()))
    {
        status
            .entries
            .push((latest.path.as_str().to_owned(), latest.revision.to_string()));
    }
    Some(status)
}

#[derive(Default)]
struct RulesSummary {
    principles: Vec<String>,
    hard_bindings: Vec<String>,
    entries: Vec<(String, String)>,
}

/// 规则来源按优先级：白名单内的 `通用工作规则.md`（母本）先于各项目的 `CLAUDE.md` 快照；
/// 硬绑定句两处都扫，因为 Skill 路由表只在 CLAUDE.md 快照里。
fn rules_summary(snapshot: &Snapshot, allow: &[VaultPath]) -> RulesSummary {
    let mut summary = RulesSummary::default();
    // 母本优先于任何项目的 CLAUDE.md 快照，与白名单顺序无关。
    let candidates = allow
        .iter()
        .map(|prefix| format!("{prefix}/通用工作规则.md"))
        .chain(allow.iter().map(|prefix| format!("{prefix}/CLAUDE.md")))
        .collect::<Vec<_>>();
    for path in candidates {
        let Some(note) = snapshot
            .notes
            .iter()
            .find(|note| note.path.as_str() == path)
        else {
            continue;
        };
        if summary.principles.is_empty() {
            summary.principles = principle_titles(&note.content);
        }
        let bindings = note
            .content
            .lines()
            .filter(|line| line.contains("一律叠加") || line.contains("必叠"))
            .map(|line| strip_markdown(line.trim_start_matches(['-', '*', '>', ' '])))
            .collect::<Vec<_>>();
        for binding in bindings {
            if !summary.hard_bindings.contains(&binding) {
                summary.hard_bindings.push(binding);
            }
        }
        summary.entries.push((path, note.revision.to_string()));
    }
    summary
}

/// 文档里第一张两列表格的「项 → 值」；只认 `| 键 | 值 |` 形态。
fn first_table_fields(content: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut in_table = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            if in_table {
                break;
            }
            continue;
        }
        in_table = true;
        let cells = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        if cells.len() != 2 || cells[0].chars().all(|c| c == '-' || c == ':') {
            continue;
        }
        if STATUS_KEYS.contains(&cells[0]) {
            fields.push((cells[0].to_owned(), strip_markdown(cells[1])));
        }
    }
    fields
}

fn section_first_bullet(content: &str, heading: &str) -> Option<String> {
    let mut in_section = false;
    for line in content.lines() {
        if line.trim_end() == heading {
            in_section = true;
            continue;
        }
        if in_section {
            if line.starts_with("## ") {
                return None;
            }
            if let Some(rest) = line.trim_start().strip_prefix("- ") {
                return Some(strip_markdown(rest));
            }
        }
    }
    None
}

fn in_progress_rows(content: &str) -> Vec<String> {
    let mut rows = Vec::new();
    let mut in_section = false;
    for line in content.lines() {
        if line.starts_with("## ") {
            in_section = line.contains("进行中");
            continue;
        }
        if !in_section || !line.trim_start().starts_with("| T-") {
            continue;
        }
        let cells = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        if cells.len() < 5 {
            continue;
        }
        rows.push(format!(
            "{} {}（{}）",
            cells[0],
            strip_markdown(cells[1]),
            strip_markdown(cells[4])
        ));
    }
    rows
}

fn unclaimed_titles(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("**待认领（")?;
            let end = rest.find("）**")?;
            Some(rest[..end].to_owned())
        })
        .collect()
}

/// 准则标题：母本形态 `## 准则① 永远说中文` 直接取标题；CLAUDE.md 形态取「准则」标题下的
/// 编号加粗条目；两者都没有时退回文件里前 8 条编号加粗条目。
fn principle_titles(content: &str) -> Vec<String> {
    let headings =
        content
            .lines()
            .filter_map(|line| line.strip_prefix("## "))
            .filter(|title| title.starts_with("准则"))
            .map(|title| {
                strip_markdown(title.trim_start_matches("准则").trim_start_matches([
                    '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨', '⑩', ' ', '·',
                ]))
            })
            .filter(|title| !title.is_empty())
            .collect::<Vec<_>>();
    if !headings.is_empty() {
        return headings;
    }
    let mut titles = Vec::new();
    let mut in_section = false;
    let mut any_section = false;
    for line in content.lines() {
        if line.starts_with("## ") {
            in_section = line.contains("准则");
            any_section |= in_section;
            continue;
        }
        if in_section && let Some(title) = numbered_bold_title(line) {
            titles.push(title);
        }
    }
    if !any_section {
        titles = content
            .lines()
            .filter_map(numbered_bold_title)
            .take(8)
            .collect();
    }
    titles
}

fn numbered_bold_title(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = trimmed[digits..].strip_prefix(". **")?;
    let end = rest.find("**")?;
    Some(rest[..end].to_owned())
}

fn strip_markdown(text: &str) -> String {
    text.replace("**", "")
        .replace("[[", "")
        .replace("]]", "")
        .replace('`', "")
        .trim()
        .to_owned()
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let head = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}
