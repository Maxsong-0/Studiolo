use studiolo_vault_index::{IndexedDocument, MarkdownDocument};

use crate::{
    ActionItem,
    markdown::{self, Line},
    metadata,
};

pub(crate) fn item(
    root: &str,
    name: &str,
    path: &str,
    line: u32,
    kind: &str,
    title: String,
) -> ActionItem {
    ActionItem {
        id: format!("{path}:{line}:{kind}"),
        project_id: root.to_owned(),
        project_name: name.to_owned(),
        kind: kind.to_owned(),
        title,
        source: markdown::location(path, line),
    }
}

pub(crate) fn tasks(root: &str, name: &str, path: &str, lines: &[Line<'_>]) -> Vec<ActionItem> {
    let mut result = Vec::new();
    for marker in ['①', '③'] {
        let Some((_, section)) = markdown::section(lines, marker) else {
            continue;
        };
        for table in markdown::tables(&section) {
            let Some(title_column) = table.column(&["任务", "改动内容", "待办", "内容"])
            else {
                continue;
            };
            let status_column = table.column(&["状态", "status"]);
            let action_column = table.column(&["需要的操作", "操作", "待用户动作"]);
            for row in table.rows {
                let Some(title) = row
                    .cells
                    .get(title_column)
                    .filter(|text| markdown::meaningful(text))
                else {
                    continue;
                };
                let status = status_column.and_then(|column| row.cells.get(column));
                if status.is_some_and(|text| markdown::completed(text))
                    || title.starts_with("[x]")
                    || title.starts_with("[X]")
                {
                    continue;
                }
                let action = action_column.and_then(|column| row.cells.get(column));
                if marker == '③'
                    && action.is_some_and(|text| {
                        let text = markdown::status_text(text);
                        markdown::completed(&text)
                            || ["已重启", "已生效", "无需重启"]
                                .iter()
                                .any(|prefix| text.starts_with(prefix))
                    })
                {
                    continue;
                }
                let kind = if marker == '③' {
                    Some("待重启")
                } else {
                    status
                        .into_iter()
                        .chain(action)
                        .find_map(|value| user_wait_kind(value))
                };
                if let Some(kind) = kind {
                    let title = action
                        .filter(|action| markdown::meaningful(action))
                        .map_or_else(|| title.clone(), |action| format!("{title} — {action}"));
                    result.push(item(root, name, path, row.line, kind, title));
                }
            }
        }
    }
    result
}

pub(crate) fn index(root: &str, name: &str, path: &str, lines: &[Line<'_>]) -> Vec<ActionItem> {
    let mut result = Vec::new();
    let mut section_level = None;
    for line in markdown::current_lines(lines) {
        if let Some((level, title)) = markdown::heading(&line) {
            if section_level.is_some_and(|start| level <= start) {
                section_level = None;
            }
            if title == "用户侧待办" || title == "待我动作" {
                section_level = Some(level);
            }
            continue;
        }
        let cleaned = markdown::clean(line.text.trim_start_matches("- ").trim_start_matches("* "));
        if let Some((label, value)) = cleaned.split_once([':', '：'])
            && matches!(label.trim(), "用户侧待办" | "待我动作")
        {
            push_todo(&mut result, root, name, path, line.number, value);
        } else if section_level.is_some()
            && let Some(value) = list_item(line.text)
        {
            push_todo(&mut result, root, name, path, line.number, value);
        }
    }
    result
}

fn list_item(text: &str) -> Option<&str> {
    text.strip_prefix("- ")
        .or_else(|| text.strip_prefix("* "))
        .or_else(|| text.strip_prefix("+ "))
        .or_else(|| {
            let (number, rest) = text.split_once(". ")?;
            (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())).then_some(rest)
        })
}

fn push_todo(
    result: &mut Vec<ActionItem>,
    root: &str,
    name: &str,
    path: &str,
    line: u32,
    text: &str,
) {
    let text = markdown::clean(text);
    if text.starts_with("[x]") || text.starts_with("[X]") || markdown::completed(&text) {
        return;
    }
    let text = text.strip_prefix("[ ]").unwrap_or(&text).trim();
    if !markdown::meaningful(text)
        || ["暂无", "无待", "没有待", "无需", "无。"]
            .iter()
            .any(|prefix| text.starts_with(prefix))
    {
        return;
    }
    result.push(item(
        root,
        name,
        path,
        line,
        descriptive_kind(text),
        text.to_owned(),
    ));
}

pub(crate) fn frontmatter(
    root: &str,
    name: &str,
    input: &MarkdownDocument,
    parsed: &IndexedDocument,
) -> Option<ActionItem> {
    let (status, line) = metadata::raw_status(input, parsed)?;
    let kind = waiting_kind(&status)?;
    Some(item(
        root,
        name,
        input.path().as_str(),
        line,
        kind,
        parsed.title.text.clone(),
    ))
}

fn user_wait_kind(text: &str) -> Option<&'static str> {
    text.split(['，', ',', '；', ';', '、', '（', '）', '(', ')'])
        .find_map(|part| {
            let part = markdown::status_text(part);
            if [
                "待用户",
                "等待用户",
                "需用户",
                "需要用户",
                "待我",
                "待外部方",
                "等待外部方",
                "待验收",
                "待拍板",
            ]
            .iter()
            .any(|prefix| part.starts_with(prefix))
            {
                waiting_kind(&part)
            } else {
                None
            }
        })
}

pub(crate) fn waiting_kind(text: &str) -> Option<&'static str> {
    let text = markdown::status_text(text);
    if [
        "待用户",
        "等待用户",
        "需用户",
        "需要用户",
        "待我",
        "待验收",
        "待拍板",
        "待评审",
        "待重启",
        "待外部方",
        "等待外部方",
        "done-pending-restart",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
    {
        Some(descriptive_kind(&text))
    } else {
        None
    }
}

fn descriptive_kind(text: &str) -> &'static str {
    if text.contains("重启") || text.starts_with("done-pending-restart") {
        "待重启"
    } else if text.contains("外部方") || text.contains("第三方") {
        "待外部方"
    } else if text.contains("验收") {
        "待验收"
    } else if ["拍板", "确认", "决策", "评审"]
        .iter()
        .any(|word| text.contains(word))
    {
        "待拍板"
    } else {
        "待用户操作"
    }
}
