//! 规则母本校验：文本 diff，删掉含约束性标记词的行记为候选扩大项。

use crate::diff::{MAX_ENTRIES, text_diff_with_removals};

use super::{Context, Expansion, Report};

const CONSTRAINT_MARKERS: [&str; 7] = ["禁止", "绝不", "不得", "一律", "必须", "永不", "必叠"];

pub(super) fn rules(report: &mut Report, canonical: &str, context: &Context) {
    let previous = context
        .current
        .as_ref()
        .map(|(_, body)| body.as_str())
        .unwrap_or("");
    let mut omitted = 0usize;
    report.diff = text_diff_with_removals(previous, canonical, |number, line| {
        if !CONSTRAINT_MARKERS
            .iter()
            .any(|marker| line.contains(marker))
        {
            return;
        }
        if report.expansions.len() == MAX_ENTRIES {
            omitted += 1;
            return;
        }
        report.expansions.push(Expansion {
            path: format!("old:{number}"),
            before: Some(line.to_owned()),
            after: None,
            note: "删除了约束性语句，可能放宽（候选，人工判断）".into(),
        });
    });
    if omitted > 0 {
        report.expansions.push(Expansion {
            path: "(additional-constraints)".into(),
            before: None,
            after: None,
            note: format!("另有 {omitted} 条约束性语句被删除；逐条展示已达上限，须审阅完整母本"),
        });
    }
}
