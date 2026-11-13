//! 固定六项路牌及不可裁剪的新鲜度。
use super::{
    ProjectStatus, STATUS_KEYS, SignpostInput,
    source::{RulesSummary, truncate},
};
use std::fmt::Write as _;

pub(super) fn render(
    input: &SignpostInput<'_>,
    projects: &[ProjectStatus],
    rules: &RulesSummary,
    level: u8,
    entry_label: &str,
) -> String {
    let focus_chars = 60usize;
    let task_chars = [80usize, 60, 40, 30][usize::from(level)];
    let max_tasks = [8usize, 5, 3, 2][usize::from(level)];
    let max_unclaimed = [3usize, 2, 1, 0][usize::from(level)];
    let digest_chars = [40usize, 30, 20, 0][usize::from(level)];
    let mut out = String::new();
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
    out.push_str("## 2 项目状态\n");
    if projects.is_empty() {
        out.push_str("白名单内没有带 00-总览/INDEX.md 的项目。\n");
    }
    if !projects.is_empty() {
        // 条数只在裁剪等级 0、1 时写进项目行（见下），说明行跟着写，不说一个行里没有的字段。
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
    out.push_str("## 4 待我做的事\n");
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
    for binding in rules.enforced.iter().take(if level >= 2 { 1 } else { 3 }) {
        let _ = writeln!(
            out,
            "硬绑定（策略强制）：{}",
            truncate(binding, if level >= 2 { 80 } else { 160 })
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
    let mut entries = 0usize;
    for project in projects {
        for (path, revision) in &project.entries {
            if entries >= max_entries {
                break;
            }
            let _ = writeln!(out, "入口：{path}@{}", &revision[..12.min(revision.len())]);
            entries += 1;
        }
    }
    if projects.len() > 1 {
        out.push_str(
            "入口规律：任一可读项目的 <项目前缀>/00-总览/INDEX.md、04-协调/TASKS.md、02-日志/<日期>.md\n",
        );
    }
    for (path, revision) in &rules.entries {
        let _ = writeln!(out, "入口：{path}@{}", &revision[..12.min(revision.len())]);
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
    out
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

    #[test]
    fn a_single_project_keeps_its_original_order() {
        let projects = [project("A", &["a1", "a2", "a3"])];
        assert_eq!(take(&projects, 2), [("A", "a1"), ("A", "a2")]);
    }
}
