//! skill 校验：摘要、触发条件与正文的结构检查；被硬绑定 skill 的正文变更记为扩大项。

use crate::{diff::json_diff, model::Skill};

use super::{Context, Expansion, Problem, Report};

pub(super) fn skill(report: &mut Report, canonical: &str, context: &Context) {
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
