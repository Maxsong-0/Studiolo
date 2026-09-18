//! desired 的结构校验、与 effective 的 diff 以及权限扩大项判定。
//!
//! 扩大项是「人必须看红的那几行」：白名单变宽、写标志变真、硬绑定被删、约束性语句被删。
//! 判定只做机器可判的部分；规则母本正文的语义放宽只能标「候选，人工判断」。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use studiolo_core::VaultPath;

use crate::{
    Diff,
    diff::{json_diff, text_diff},
    model::{self, CapabilitySet, PolicyKind, Skill, sha256_hex},
};

/// 一条校验问题：位置与原因（不回显正文）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// 字段路径或行号。
    pub path: String,
    /// 原因。
    pub message: String,
}

impl Problem {
    pub(crate) fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// 一条权限扩大项（或收窄项）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expansion {
    /// 字段路径或行号。
    pub path: String,
    /// 改前。
    pub before: Option<String>,
    /// 改后。
    pub after: Option<String>,
    /// 为什么算扩大。
    pub note: String,
}

/// 校验报告：结构问题、diff、扩大项、收窄项与校验时的 effective 基线。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// 是否可发布。
    pub ok: bool,
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 是否为撤销（retire）该 subject。
    pub retire: bool,
    /// 结构问题；非空即不可发布。
    pub problems: Vec<Problem>,
    /// 与当前 effective 的差异。
    pub diff: Diff,
    /// 权限扩大项（界面标红）。
    pub expansions: Vec<Expansion>,
    /// 权限收窄项。
    pub narrowings: Vec<Expansion>,
    /// 校验所基于的 effective 哈希。
    pub base_effective_hash: String,
    /// 该 subject 当前生效版本（无则为空）。
    pub current_version: Option<u64>,
    /// 规范化正文的 sha256（撤销时为空串的 sha256）。
    pub body_sha256: String,
    #[serde(skip)]
    pub(crate) canonical: Option<String>,
}

/// 校验所需的 effective 上下文（在同一事务里读出）。
pub(crate) struct Context {
    /// 该 subject 当前版本与正文。
    pub current: Option<(u64, String)>,
    /// 当前 effective 哈希。
    pub effective_hash: String,
    /// 已发布的 skill 名。
    pub skills: BTreeSet<String>,
    /// skill → 硬绑定它的 AI 类型。
    pub bindings: BTreeMap<String, Vec<String>>,
}

const CONSTRAINT_MARKERS: [&str; 7] = ["禁止", "绝不", "不得", "一律", "必须", "永不", "必叠"];

pub(crate) fn validate(
    kind: PolicyKind,
    subject: &str,
    body: Option<&str>,
    limit: usize,
    context: &Context,
) -> Report {
    let mut report = Report {
        ok: false,
        kind,
        subject: subject.to_owned(),
        retire: body.is_none(),
        problems: Vec::new(),
        diff: Diff::empty(match kind {
            PolicyKind::Rules => "text",
            _ => "json",
        }),
        expansions: Vec::new(),
        narrowings: Vec::new(),
        base_effective_hash: context.effective_hash.clone(),
        current_version: context.current.as_ref().map(|(version, _)| *version),
        body_sha256: sha256_hex(b""),
        canonical: None,
    };
    if model::validate_subject(kind, subject).is_err() {
        report
            .problems
            .push(Problem::new("subject", "subject 不满足该类别的命名规则"));
        return report;
    }
    let Some(body) = body else {
        retire(&mut report, context);
        report.ok = report.problems.is_empty();
        return report;
    };
    let canonical = match model::canonicalize(kind, subject, body, limit) {
        Ok(text) => text,
        Err(problem) => {
            report.problems.push(problem);
            return report;
        }
    };
    report.body_sha256 = sha256_hex(canonical.as_bytes());
    match kind {
        PolicyKind::CapabilitySet => capability_set(&mut report, &canonical, context),
        PolicyKind::Skills => skill(&mut report, &canonical, context),
        PolicyKind::Rules => rules(&mut report, &canonical, context),
    }
    report.ok = report.problems.is_empty();
    report.canonical = Some(canonical);
    report
}

fn retire(report: &mut Report, context: &Context) {
    let Some((_, current)) = &context.current else {
        report
            .problems
            .push(Problem::new("subject", "该 subject 没有生效版本，无可撤销"));
        return;
    };
    if report.kind == PolicyKind::Skills
        && let Some(types) = context.bindings.get(&report.subject)
    {
        report.problems.push(Problem::new(
            "subject",
            format!(
                "该 skill 仍被能力集硬绑定（{}），先解除绑定",
                types.join("、")
            ),
        ));
    }
    report.diff = match report.kind {
        PolicyKind::Rules => text_diff(current, ""),
        _ => json_diff(serde_json::from_str(current).ok().as_ref(), None),
    };
    report.expansions.push(Expansion {
        path: "(retire)".into(),
        before: Some(format!("version {}", report.current_version.unwrap_or(0))),
        after: None,
        note: match report.kind {
            PolicyKind::Rules => "撤销整份规则母本：全部约束消失，需人工确认".into(),
            PolicyKind::Skills => "撤销 skill：索引候选消失".into(),
            PolicyKind::CapabilitySet => "撤销能力集：该类型回到零权限（安全方向）".into(),
        },
    });
}

fn capability_set(report: &mut Report, canonical: &str, context: &Context) {
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
    if set.write.board {
        report.problems.push(Problem::new(
            "write.board",
            "M5 黑板写入尚未开放，此项恒为 false",
        ));
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
    let flags = [
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

fn skill(report: &mut Report, canonical: &str, context: &Context) {
    let Ok(skill) = serde_json::from_str::<Skill>(canonical) else {
        report.problems.push(Problem::new("body", "skill 无法解析"));
        return;
    };
    if skill.summary.trim().is_empty() || skill.summary.chars().count() > 200 {
        report
            .problems
            .push(Problem::new("summary", "摘要须为 1..=200 字符"));
    }
    if skill.triggers.len() > 16 {
        report
            .problems
            .push(Problem::new("triggers", "触发条件最多 16 条"));
    }
    for (index, trigger) in skill.triggers.iter().enumerate() {
        if trigger.trim().is_empty() || trigger.chars().count() > 200 {
            report.problems.push(Problem::new(
                format!("triggers[{index}]"),
                "触发条件须为 1..=200 字符",
            ));
        }
    }
    if skill.markdown.trim().is_empty() {
        report
            .problems
            .push(Problem::new("markdown", "skill 正文不能为空"));
    }
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
    if let Some(types) = context.bindings.get(&report.subject)
        && context
            .current
            .as_ref()
            .is_some_and(|(_, body)| body != canonical)
    {
        report.expansions.push(Expansion {
            path: "markdown".into(),
            before: None,
            after: None,
            note: format!(
                "被硬绑定的 skill 正文变更（绑定方：{}），请人工判断",
                types.join("、")
            ),
        });
    }
}

fn rules(report: &mut Report, canonical: &str, context: &Context) {
    let previous = context
        .current
        .as_ref()
        .map(|(_, body)| body.as_str())
        .unwrap_or("");
    report.diff = text_diff(previous, canonical);
    for entry in &report.diff.entries {
        if entry.op != crate::DiffOp::Removed {
            continue;
        }
        let Some(line) = &entry.before else {
            continue;
        };
        if CONSTRAINT_MARKERS
            .iter()
            .any(|marker| line.contains(marker))
        {
            report.expansions.push(Expansion {
                path: entry.path.clone(),
                before: Some(line.clone()),
                after: None,
                note: "删除了约束性语句，可能放宽（候选，人工判断）".into(),
            });
        }
    }
}
