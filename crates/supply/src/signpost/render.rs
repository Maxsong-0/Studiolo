//! 固定六项路牌及不可裁剪的新鲜度。
//!
//! 节号固定（别处按「第 5 项」这样引用），排列按「只看得到开头一段的读者最先要什么」：1、5、6、3、4、2。
//! 客户端会从尾部截 MCP `instructions`（`CLIENT_VISIBLE_UTF16`），放在最后的只能是项目清单（vault BUG-061）。
use super::{
    ProjectStatus, STATUS_KEYS, SignpostInput,
    source::{RulesSummary, truncate},
};
use crate::budget::utf16_len;
use std::fmt::Write as _;

/// 两个独立的裁剪等级（0 = 不裁，3 = 最深）。
#[derive(Clone, Copy)]
pub(super) struct Trim {
    /// 可见段：第 5、6、4 节。
    pub(super) front: u8,
    /// 第 2 节的项目清单。
    pub(super) projects: u8,
}

/// 一次渲染的结果。
pub(super) struct Rendered {
    pub(super) text: String,
    /// 可见段（到第 2 节的说明行为止，只剩项目清单在后面）有多少个 UTF-16 单元。
    pub(super) front_utf16: usize,
    /// 第 4 节标题下一行的字节位置，黑板行插在这里。
    pub(super) board_at: usize,
}

pub(super) fn render(
    input: &SignpostInput<'_>,
    projects: &[ProjectStatus],
    rules: &RulesSummary,
    trim: Trim,
    entry_label: &str,
) -> Rendered {
    let mut out = String::new();
    push_identity(&mut out, input);
    push_constraints(&mut out, rules, trim.front);
    push_entries(&mut out, input, projects, rules, trim.front, entry_label);
    push_capability(&mut out, input);
    out.push_str("## 4 待我做的事\n");
    let board_at = out.len();
    push_todo(&mut out, projects, trim.front);
    push_projects_head(&mut out, input, projects, trim.projects);
    let front_utf16 = utf16_len(&out);
    push_project_list(&mut out, projects, trim.projects);
    Rendered {
        text: out,
        front_utf16,
        board_at,
    }
}

/// 抬头、新鲜度与第 1 节；部署验收和客户端投影按固定行号认这几行，新内容只能接在「类型」行后面。
fn push_identity(out: &mut String, input: &SignpostInput<'_>) {
    let _ = writeln!(
        out,
        "# Studiolo 路牌（代次 {}，{}）",
        input.generation, input.generated_at
    );
    if let Some(freshness) = input.freshness {
        let _ = writeln!(out, "{freshness}");
    }
    let _ = writeln!(
        out,
        "## 1 我是谁\n类型 {} · 实例 {} · 注册 {}",
        input.principal.agent_type, input.principal.instance_id, input.principal.registered_at
    );
    out.push_str(
        "看法：各节按 1、5、6、3、4、2 排，要紧的在前；末尾出现 [truncated] 说明被客户端截了，调 handshake 取全文。\n",
    );
}

fn push_constraints(out: &mut String, rules: &RulesSummary, level: u8) {
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
    let (max_bindings, binding_chars) = if level >= 2 { (1, 80) } else { (3, 160) };
    for binding in rules.enforced.iter().take(max_bindings) {
        let _ = writeln!(
            out,
            "硬绑定（策略强制）：{}",
            truncate(binding, binding_chars)
        );
    }
    for binding in rules.hard_bindings.iter().take(max_bindings) {
        let _ = writeln!(out, "硬绑定：{}", truncate(binding, binding_chars));
    }
}

fn push_entries(
    out: &mut String,
    input: &SignpostInput<'_>,
    projects: &[ProjectStatus],
    rules: &RulesSummary,
    level: u8,
    entry_label: &str,
) {
    out.push_str("## 6 继续拉\n");
    let _ = writeln!(
        out,
        "{entry_label}：{}",
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
    let max_entries = [12usize, 8, 4, 2][usize::from(level)];
    for (path, revision) in projects
        .iter()
        .flat_map(|project| &project.entries)
        .take(max_entries)
    {
        push_entry(out, path, revision);
    }
    let has_pattern = projects.len() > 1;
    if has_pattern {
        out.push_str(
            "入口规律：任一可读项目的 <项目前缀>/00-总览/INDEX.md、04-协调/TASKS.md、02-日志/<日期>.md、CLAUDE.md\n",
        );
    }
    // 「入口规律」说得清的不再逐条列：一个项目一行的 CLAUDE.md 随项目数涨，会把后面几节推出可见段。
    let in_pattern = |path: &str| {
        has_pattern
            && path
                .strip_suffix("/CLAUDE.md")
                .is_some_and(|prefix| projects.iter().any(|project| project.prefix == prefix))
    };
    for (path, revision) in rules.entries.iter().filter(|(path, _)| !in_pattern(path)) {
        push_entry(out, path, revision);
    }
    if let Some(policy) = input.policy
        && !policy.skills.is_empty()
    {
        let _ = writeln!(
            out,
            "skills：{} 个（policy_read kind=skills 取索引）",
            policy.skills.len()
        );
    }
    if !input.snapshot.diagnostics.is_empty() {
        let _ = writeln!(
            out,
            "诊断：{} 条扫描诊断，视图可能不完整",
            input.snapshot.diagnostics.len()
        );
    }
}

fn push_entry(out: &mut String, path: &str, revision: &str) {
    let _ = writeln!(out, "入口：{path}@{}", &revision[..12.min(revision.len())]);
}

fn push_capability(out: &mut String, input: &SignpostInput<'_>) {
    match input.policy {
        Some(policy) => {
            let prefixes = policy
                .read_prefixes
                .iter()
                .map(|prefix| prefix.as_str())
                .collect::<Vec<_>>()
                .join("、");
            let _ = writeln!(
                out,
                "## 3 能力\n读：{}（能力集 v{} ∩ 部署上限）；写：黑板 {}、正文 {}；策略：{}；策略版本 {}",
                if prefixes.is_empty() {
                    "无"
                } else {
                    &prefixes
                },
                policy.capability_version,
                if policy.board_write {
                    "可写"
                } else {
                    "只读"
                },
                if policy.content_patch_only {
                    "仅经预览的 patch"
                } else {
                    "无"
                },
                if policy.propose_desired {
                    "可提 desired（永不自动生效）"
                } else {
                    "不可提 desired"
                },
                &policy.effective_hash[..12.min(policy.effective_hash.len())]
            );
        }
        None => {
            let _ = writeln!(
                out,
                "## 3 能力\n只读；白名单：{}；写与审批：无（M3）；策略平面：未接入",
                input
                    .allow
                    .iter()
                    .map(|prefix| prefix.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            );
        }
    }
}

/// 第 4 节标题以下的行；标题由 [`render`] 先写，好记下黑板行的位置。
fn push_todo(out: &mut String, projects: &[ProjectStatus], level: u8) {
    let task_chars = [80usize, 60, 40, 30][usize::from(level)];
    let max_tasks = [8usize, 5, 3, 2][usize::from(level)];
    let max_unclaimed = [3usize, 2, 1, 0][usize::from(level)];
    let in_progress = round_robin(
        projects,
        |project| project.in_progress.as_slice(),
        max_tasks,
    );
    let unclaimed = round_robin(
        projects,
        |project| project.unclaimed.as_slice(),
        max_unclaimed,
    );
    for (prefix, row) in &in_progress {
        let row = truncate(row, task_chars + 40);
        let _ = writeln!(out, "进行中：{prefix} · {row}");
    }
    for (prefix, title) in &unclaimed {
        let title = truncate(title, task_chars);
        let _ = writeln!(out, "待认领：{prefix} · {title}");
    }
    let total_in_progress = projects
        .iter()
        .map(|project| project.in_progress.len())
        .sum::<usize>();
    let total_unclaimed = projects
        .iter()
        .map(|project| project.unclaimed.len())
        .sum::<usize>();
    // 按合计判断而不是按列出的条数：最深裁剪不列待认领时，也不能说成「无」。
    if total_in_progress + total_unclaimed > 0 {
        let _ = writeln!(
            out,
            "（共 {total_in_progress} 条进行中、{total_unclaimed} 条待认领，这里列出 {}＋{} 条；各项目条数见第 2 节，完整见各项目 04-协调/TASKS.md）",
            in_progress.len(),
            unclaimed.len()
        );
    } else {
        out.push_str("无（协调板无进行中或待认领项，或不可用）\n");
    }
}

/// 第 2 节标题、容量提醒与说明行：算在可见段里，容量提醒要让 AI 看到并转告用户。
fn push_projects_head(
    out: &mut String,
    input: &SignpostInput<'_>,
    projects: &[ProjectStatus],
    level: u8,
) {
    out.push_str("## 2 项目状态\n");
    if let Some(capacity) = input.capacity {
        let _ = writeln!(out, "{capacity}");
    }
    if projects.is_empty() {
        out.push_str("白名单内没有带 00-总览/INDEX.md 的项目。\n");
        return;
    }
    // 条数只在裁剪等级 0、1 时写进项目行（见 `push_project_list`），说明行跟着写，不说一个行里没有的字段。
    let counts = if level <= 1 {
        " · 进行中／待认领条数（有才写）"
    } else {
        ""
    };
    let _ = writeln!(
        out,
        "{} 个可读项目；每行＝状态 · 未关闭 BUG{counts}，详情读 <项目前缀>/00-总览/INDEX.md",
        projects.len()
    );
}

fn push_project_list(out: &mut String, projects: &[ProjectStatus], level: u8) {
    let focus_chars = 60usize;
    let digest_chars = [40usize, 30, 20, 0][usize::from(level)];
    for project in projects {
        if level >= 3 {
            let _ = writeln!(out, "- {}", project.prefix);
            continue;
        }
        let field = |key: &str| {
            project
                .fields
                .iter()
                .find(|(name, _)| name == key)
                .map_or("不可用", |(_, value)| value.as_str())
        };
        let _ = write!(
            out,
            "- {}：{} · 未关闭 BUG {}",
            project.prefix,
            truncate(field(STATUS_KEYS[1]), digest_chars),
            truncate(field(STATUS_KEYS[3]), 24)
        );
        // 第 4 节只轮流列每个项目的前几条，各项目实际有多少在这里补齐。
        let pending = project.in_progress.len() + project.unclaimed.len();
        if level <= 1 && pending > 0 {
            let _ = write!(
                out,
                " · 进行中 {}／待认领 {}",
                project.in_progress.len(),
                project.unclaimed.len()
            );
        }
        if level == 0
            && let Some(focus) = project.focus.as_deref()
        {
            let _ = write!(out, " · 重点：{}", truncate(focus, focus_chars));
        }
        let _ = writeln!(out);
    }
}

/// 按项目轮流取：第 1 轮每个项目取它的第 1 条，第 2 轮取第 2 条……取满 `limit` 或取完为止。
///
/// 按项目顺序取满就停的话，项目一多，只有排在最前的两三个能出现在路牌里。
/// 返回（项目前缀，行）对，顺序就是取的先后。
fn round_robin<'a>(
    projects: &'a [ProjectStatus],
    rows: impl Fn(&'a ProjectStatus) -> &'a [String],
    limit: usize,
) -> Vec<(&'a str, &'a str)> {
    let rounds = projects
        .iter()
        .map(|project| rows(project).len())
        .max()
        .unwrap_or(0);
    let mut picked = Vec::new();
    for round in 0..rounds {
        for project in projects {
            if picked.len() >= limit {
                return picked;
            }
            if let Some(row) = rows(project).get(round) {
                picked.push((project.prefix.as_str(), row.as_str()));
            }
        }
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::{ProjectStatus, round_robin};

    fn project(prefix: &str, in_progress: &[&str]) -> ProjectStatus {
        ProjectStatus {
            prefix: prefix.to_owned(),
            in_progress: in_progress.iter().map(|row| (*row).to_owned()).collect(),
            ..ProjectStatus::default()
        }
    }

    fn take(projects: &[ProjectStatus], limit: usize) -> Vec<(&str, &str)> {
        round_robin(projects, |project| project.in_progress.as_slice(), limit)
    }

    #[test]
    fn rounds_visit_every_project_before_taking_second_rows() {
        let projects = [
            project("A", &["a1", "a2", "a3"]),
            project("B", &[]),
            project("C", &["c1"]),
            project("D", &["d1", "d2"]),
        ];
        assert_eq!(
            take(&projects, 5),
            [
                ("A", "a1"),
                ("C", "c1"),
                ("D", "d1"),
                ("A", "a2"),
                ("D", "d2"),
            ]
        );
        assert_eq!(take(&projects, 2), [("A", "a1"), ("C", "c1")]);
        assert_eq!(take(&projects, 100).len(), 6);
        assert!(take(&projects, 0).is_empty());
        assert!(take(&[], 8).is_empty());
    }

    /// 容量提醒和新鲜度一样不可裁剪：每个裁剪等级都在第 2 节开头（T-215 复查）。
    #[test]
    fn capacity_notice_opens_section_two_at_every_trim_level() {
        use super::{RulesSummary, SignpostInput, Trim, render};
        use crate::{Snapshot, signpost::Principal};
        let snapshot = Snapshot::default();
        let principal = Principal {
            agent_type: "claude".into(),
            instance_id: "01JT3STUDIOLO0000000000001".into(),
            registered_at: "2026-09-07T01:00:00Z".into(),
        };
        let capacity = "⚠ 副本快到协议上限：Markdown 合计 32.0 MiB，上限 40.0 MiB 的 80%。";
        let input = SignpostInput {
            snapshot: &snapshot,
            allow: &[],
            principal: &principal,
            generation: 1,
            generated_at: "2026-09-07T01:00:00Z",
            freshness: Some("副本状态 ready"),
            capacity: Some(capacity),
            tools: &[],
            policy: None,
            rules_whitelist: None,
        };
        let projects = [project("项目/A", &["a1"])];
        for level in 0..=3 {
            let trim = Trim {
                front: level,
                projects: level,
            };
            let text = render(&input, &projects, &RulesSummary::default(), trim, "tools").text;
            assert!(
                text.contains(&format!("## 2 项目状态\n{capacity}\n")),
                "裁剪等级 {level}：\n{text}"
            );
        }
    }

    #[test]
    fn a_single_project_keeps_its_original_order() {
        let projects = [project("A", &["a1", "a2", "a3"])];
        assert_eq!(take(&projects, 2), [("A", "a1"), ("A", "a2")]);
    }
}
