//! 能力集校验：恒定约束与路径合法性，以及相对现行版本的扩大项与收窄项。

use std::collections::BTreeSet;

use studiolo_core::VaultPath;

use crate::{
    diff::json_diff,
    model::{CapabilitySet, validate_rules_subject},
};

use super::{Context, Expansion, Problem, Report};

pub(super) fn capability_set(report: &mut Report, canonical: &str, context: &Context) {
    let Ok(set) = serde_json::from_str::<CapabilitySet>(canonical) else {
        report.problems.push(Problem::new("body", "能力集无法解析"));
        return;
    };
    if !set.read.deny_by_default {
        report.problems.push(Problem::new(
            "read.deny_by_default",
            "必须为 true：白名单之外不进入计算",
        ));
    }
    if set.write.content_direct {
        report.problems.push(Problem::new(
            "write.content_direct",
            "永久为 false：AI 不整页覆盖人写正文",
        ));
    }
    if set.write.board && set.write.board_projects.is_empty() {
        report.problems.push(Problem::new(
            "write.board",
            "黑板写入必须有非空显式 write.board_projects",
        ));
    }
    if !set.write.board && !set.write.board_projects.is_empty() {
        report.problems.push(Problem::new(
            "write.board_projects",
            "黑板只读时写项目必须为空",
        ));
    }
    let mut write_seen = BTreeSet::new();
    for prefix in &set.write.board_projects {
        let valid = VaultPath::parse(prefix).is_ok_and(|p| {
            !p.is_protected()
                && p.as_str().starts_with("项目/")
                && p.as_str().split('/').count() == 2
        });
        if !valid
            || !covered(prefix, &set.read.project_whitelist)
            || !write_seen.insert(prefix.clone())
        {
            report.problems.push(Problem::new(
                "write.board_projects",
                "必须为唯一、合法且位于读取白名单内的完整项目路径",
            ));
        }
    }
    if set.policy.activate {
        report.problems.push(Problem::new(
            "policy.activate",
            "永久为 false：AI 无 One-time Code 路径",
        ));
    }
    let mut seen = BTreeSet::new();
    for (index, prefix) in set.read.project_whitelist.iter().enumerate() {
        let path = format!("read.project_whitelist[{index}]");
        match VaultPath::parse(prefix) {
            Ok(parsed) if parsed.is_protected() => {
                report
                    .problems
                    .push(Problem::new(path, "受保护目录不能进白名单"));
            }
            Ok(_) => {
                if !seen.insert(prefix.clone()) {
                    report.problems.push(Problem::new(path, "重复的白名单前缀"));
                }
            }
            Err(_) => report
                .problems
                .push(Problem::new(path, "不是合法的逻辑路径前缀")),
        }
    }
    // 规则名单的每项都须是合法的规则母本 subject（与策略平面 rules 类同一命名规则，ADR-026 D3）。
    let mut rules_seen = BTreeSet::new();
    for (index, subject) in set.read.rules_whitelist.iter().enumerate() {
        let path = format!("read.rules_whitelist[{index}]");
        if validate_rules_subject(subject).is_err() {
            report
                .problems
                .push(Problem::new(path, "不是合法的规则 subject"));
        } else if !rules_seen.insert(subject.as_str()) {
            report
                .problems
                .push(Problem::new(path, "重复的规则 subject"));
        }
    }
    for (index, binding) in set.hard_bound_skills.iter().enumerate() {
        let path = format!("hard_bound_skills[{index}]");
        if binding.when.trim().is_empty() || binding.when.chars().count() > 200 {
            report.problems.push(Problem::new(
                format!("{path}.when"),
                "触发条件须为 1..=200 字符",
            ));
        }
        if !context.skills.contains(&binding.must_use) {
            report.problems.push(Problem::new(
                format!("{path}.must_use"),
                "指向的 skill 尚未发布",
            ));
        }
    }
    let previous = context
        .current
        .as_ref()
        .and_then(|(_, body)| serde_json::from_str::<CapabilitySet>(body).ok());
    report.diff = json_diff(
        context
            .current
            .as_ref()
            .and_then(|(_, body)| serde_json::from_str::<serde_json::Value>(body).ok())
            .as_ref(),
        serde_json::from_str::<serde_json::Value>(canonical)
            .ok()
            .as_ref(),
    );
    let old_whitelist = previous
        .as_ref()
        .map(|set| set.read.project_whitelist.clone())
        .unwrap_or_default();
    for prefix in &set.read.project_whitelist {
        if !covered(prefix, &old_whitelist) {
            report.expansions.push(Expansion {
                path: "read.project_whitelist".into(),
                before: None,
                after: Some(prefix.clone()),
                note: if previous.is_none() {
                    "新类型首次获得读取范围".into()
                } else {
                    "读取范围变宽".into()
                },
            });
        }
    }
    for prefix in &old_whitelist {
        if !covered(prefix, &set.read.project_whitelist) {
            report.narrowings.push(Expansion {
                path: "read.project_whitelist".into(),
                before: Some(prefix.clone()),
                after: None,
                note: "读取范围收窄".into(),
            });
        }
    }
    let old_rules = previous
        .as_ref()
        .map(|set| set.read.rules_whitelist.clone())
        .unwrap_or_default();
    for subject in &set.read.rules_whitelist {
        if !old_rules.contains(subject) {
            report.expansions.push(Expansion {
                path: "read.rules_whitelist".into(),
                before: None,
                after: Some(subject.clone()),
                note: if previous.is_none() {
                    "新类型首次获得规则读取".into()
                } else {
                    "规则读取范围变宽".into()
                },
            });
        }
    }
    for subject in &old_rules {
        if !set.read.rules_whitelist.contains(subject) {
            report.narrowings.push(Expansion {
                path: "read.rules_whitelist".into(),
                before: Some(subject.clone()),
                after: None,
                note: "规则读取范围收窄".into(),
            });
        }
    }
    let old_board = previous
        .as_ref()
        .map(|s| s.write.board_projects.clone())
        .unwrap_or_default();
    for prefix in &set.write.board_projects {
        if !old_board.contains(prefix) {
            report.expansions.push(Expansion {
                path: "write.board_projects".into(),
                before: None,
                after: Some(prefix.clone()),
                note: "获得该项目黑板写权限".into(),
            });
        }
    }
    for prefix in &old_board {
        if !set.write.board_projects.contains(prefix) {
            report.narrowings.push(Expansion {
                path: "write.board_projects".into(),
                before: Some(prefix.clone()),
                after: None,
                note: "收回该项目黑板写权限".into(),
            });
        }
    }
    let flags = [
        (
            "write.board",
            previous.as_ref().is_some_and(|s| s.write.board),
            set.write.board,
            "获得黑板写权限",
        ),
        (
            "write.content_patch_only",
            previous
                .as_ref()
                .is_some_and(|set| set.write.content_patch_only),
            set.write.content_patch_only,
            "获得以 patch 形式写正文的能力",
        ),
        (
            "policy.propose_desired",
            previous
                .as_ref()
                .is_some_and(|set| set.policy.propose_desired),
            set.policy.propose_desired,
            "获得提交 desired 提案的能力（永不自动生效）",
        ),
    ];
    for (path, before, after, note) in flags {
        if !before && after {
            report.expansions.push(Expansion {
                path: path.into(),
                before: Some("false".into()),
                after: Some("true".into()),
                note: note.into(),
            });
        } else if before && !after {
            report.narrowings.push(Expansion {
                path: path.into(),
                before: Some("true".into()),
                after: Some("false".into()),
                note: "能力收回".into(),
            });
        }
    }
    let old_bindings = previous
        .as_ref()
        .map(|set| set.hard_bound_skills.clone())
        .unwrap_or_default();
    for binding in &old_bindings {
        if !set.hard_bound_skills.contains(binding) {
            report.expansions.push(Expansion {
                path: "hard_bound_skills".into(),
                before: Some(format!("{} → {}", binding.when, binding.must_use)),
                after: None,
                note: "硬绑定被解除：约束放宽".into(),
            });
        }
    }
    for binding in &set.hard_bound_skills {
        if !old_bindings.contains(binding) {
            report.narrowings.push(Expansion {
                path: "hard_bound_skills".into(),
                before: None,
                after: Some(format!("{} → {}", binding.when, binding.must_use)),
                note: "新增硬绑定：约束收紧".into(),
            });
        }
    }
}

/// `prefix` 是否已被 `set` 里某个相同或更宽的前缀覆盖。
fn covered(prefix: &str, set: &[String]) -> bool {
    set.iter()
        .any(|existing| existing == prefix || prefix.starts_with(&format!("{existing}/")))
}
