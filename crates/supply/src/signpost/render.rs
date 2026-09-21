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
        let _ = writeln!(
            out,
            "{} 个可读项目；每行＝状态 · 未关闭 BUG，详情读 <项目前缀>/00-总览/INDEX.md",
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
    let mut any = false;
    let mut in_progress = 0usize;
    let mut unclaimed = 0usize;
    for project in projects {
        for row in &project.in_progress {
            if in_progress >= max_tasks {
                break;
            }
            let _ = writeln!(
                out,
                "进行中：{} · {}",
                project.prefix,
                truncate(row, task_chars + 40)
            );
            in_progress += 1;
            any = true;
        }
    }
    for project in projects {
        for title in &project.unclaimed {
            if unclaimed >= max_unclaimed {
                break;
            }
            let _ = writeln!(
                out,
                "待认领：{} · {}",
                project.prefix,
                truncate(title, task_chars)
            );
            unclaimed += 1;
            any = true;
        }
    }
    if any {
        out.push_str("（仅列前若干条；完整见各项目 04-协调/TASKS.md）\n");
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
