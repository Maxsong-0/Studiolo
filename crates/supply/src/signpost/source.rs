//! 从白名单结构化 Markdown 派生状态与规则，不读取手写路牌。
use super::{ProjectStatus, STATUS_KEYS};
use crate::Snapshot;
use studiolo_core::VaultPath;

pub(super) fn project_status(snapshot: &Snapshot, prefix: &VaultPath) -> Option<ProjectStatus> {
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
pub(super) struct RulesSummary {
    pub(super) principles: Vec<String>,
    pub(super) hard_bindings: Vec<String>,
    pub(super) entries: Vec<(String, String)>,
}

/// 规则来源按优先级：白名单内的 `通用工作规则.md`（母本）先于各项目的 `CLAUDE.md` 快照；
/// 硬绑定句两处都扫，因为 Skill 路由表只在 CLAUDE.md 快照里。
pub(super) fn rules_summary(snapshot: &Snapshot, allow: &[VaultPath]) -> RulesSummary {
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

pub(super) fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let head = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}
