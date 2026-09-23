//! desired 的结构校验、与 effective 的 diff 以及权限扩大项判定。
//!
//! 扩大项是「人必须看红的那几行」：白名单变宽、写标志变真、硬绑定被删、约束性语句被删。
//! 判定只做机器可判的部分；规则母本正文的语义放宽只能标「候选，人工判断」。

mod capability;
mod rules;
mod skill;
mod types;

use crate::{
    Diff,
    diff::{json_diff, text_diff},
    model::{self, PolicyKind, sha256_hex},
};

use capability::capability_set;
use rules::rules;
use skill::skill;
pub(crate) use types::Context;
pub use types::{Expansion, Problem, Report};

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
