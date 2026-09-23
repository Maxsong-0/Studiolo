//! 规则母本校验：文本 diff，删掉含约束性标记词的行记为候选扩大项。

use crate::diff::text_diff;

use super::{Context, Expansion, Report};

const CONSTRAINT_MARKERS: [&str; 7] = ["禁止", "绝不", "不得", "一律", "必须", "永不", "必叠"];

pub(super) fn rules(report: &mut Report, canonical: &str, context: &Context) {
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
