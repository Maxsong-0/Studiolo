use studiolo_vault_index::{FrontmatterValue, NormalizedStatus};

use crate::{
    ActionItem, Sourced, actions,
    derive::{Inputs, direct_child},
    markdown, metadata,
};

pub(crate) fn open_bugs(
    root: &str,
    name: &str,
    inputs: &Inputs<'_>,
    diagnostics: &mut Vec<String>,
    inbox: &mut Vec<ActionItem>,
) -> Sourced<u64> {
    let prefix = format!("{root}/06-BUG/");
    let mut count = 0;
    let mut source = None;
    let mut unknown = false;
    for (path, (input, parsed)) in inputs
        .iter()
        .filter(|(path, _)| direct_child(path, &prefix))
    {
        if let Some(action) = actions::frontmatter(root, name, input, parsed) {
            inbox.push(action);
        }
        let status = metadata::value(input, parsed, &["status"]);
        let classified = status.and_then(|(value, line)| bug_open(value).map(|open| (open, line)));
        if let Some((open, line)) = classified {
            source.get_or_insert_with(|| markdown::location(path, line));
            count += u64::from(open);
        } else {
            unknown = true;
            diagnostics.push(format!("{path}: BUG 状态未知，未关闭 BUG 总数不展示为 0"));
        }
    }
    if unknown {
        return Sourced::unknown();
    }
    source.map_or_else(
        || {
            diagnostics.push("未关闭 BUG 数未知：没有可核验的 BUG 状态来源".to_owned());
            Sourced::unknown()
        },
        |source| Sourced::known(count, source),
    )
}

fn bug_open(value: &FrontmatterValue) -> Option<bool> {
    match value {
        FrontmatterValue::Status(
            NormalizedStatus::Fixed
            | NormalizedStatus::Done
            | NormalizedStatus::Superseded
            | NormalizedStatus::Deprecated,
        ) => Some(false),
        FrontmatterValue::Status(
            NormalizedStatus::InProgress | NormalizedStatus::Pending | NormalizedStatus::Draft,
        ) => Some(true),
        FrontmatterValue::Status(NormalizedStatus::Unknown(raw)) => {
            let status = markdown::status_text(raw);
            if [
                "未修复",
                "待修复",
                "未关闭",
                "待处理",
                "处理中",
                "未解决",
                "open",
                "unfixed",
                "in_progress",
            ]
            .contains(&status.as_str())
                || actions::waiting_kind(&status).is_some()
            {
                Some(true)
            } else if ["已关闭", "已解决", "closed", "fixed", "resolved", "wontfix"]
                .contains(&status.as_str())
            {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
    }
}
