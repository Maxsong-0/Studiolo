use std::collections::{BTreeMap, BTreeSet};

use studiolo_vault_index::{IndexedDocument, MarkdownDocument};

use crate::{ActionItem, ProjectSummary, Sourced, actions, bugs, markdown, metadata, scan};

pub(crate) type Inputs<'a> = BTreeMap<String, (&'a MarkdownDocument, IndexedDocument)>;

pub(crate) fn build(inputs: &Inputs<'_>) -> (Vec<ProjectSummary>, Vec<ActionItem>) {
    let names: BTreeSet<_> = inputs
        .keys()
        .filter_map(|path| {
            let (name, remainder) = path.strip_prefix("项目/")?.split_once('/')?;
            (!name.is_empty() && !remainder.is_empty()).then_some(name.to_owned())
        })
        .collect();
    let mut projects = Vec::new();
    let mut inbox = Vec::new();
    for name in names {
        let root = format!("项目/{name}");
        let index_path = format!("{root}/00-总览/INDEX.md");
        let tasks_path = format!("{root}/04-协调/TASKS.md");
        let roadmap_path = format!("{root}/05-版本/roadmap.md");
        let declared = [&index_path, &tasks_path, &roadmap_path]
            .iter()
            .filter(|path| inputs.contains_key(path.as_str()))
            .count();
        let mut project = ProjectSummary {
            id: root.clone(),
            name: name.clone(),
            root: root.clone(),
            lifecycle: Sourced::unknown(),
            version: Sourced::unknown(),
            active_tasks: Sourced::unknown(),
            open_bugs: Sourced::unknown(),
            next_milestone: Sourced::unknown(),
            profile: match declared {
                0 => "unfiled",
                3 => "standard",
                _ => "partial",
            }
            .to_owned(),
            source: None,
            diagnostics: Vec::new(),
        };
        if let Some((input, parsed)) = inputs.get(&index_path) {
            project.source = Some(markdown::location(&index_path, 1));
            let lines = markdown::current_lines(&markdown::lines(input.contents()));
            let lifecycle = metadata::text(input, parsed, &["lifecycle", "生命周期"])
                .or_else(|| markdown::field(&lines, &["lifecycle", "生命周期"]));
            if let Some((value, line)) = lifecycle {
                if ["活跃", "暂停", "已交付维护", "静态资料", "候选", "未建档"]
                    .contains(&value.as_str())
                {
                    project.lifecycle =
                        Sourced::known(value, markdown::location(&index_path, line));
                } else {
                    project
                        .diagnostics
                        .push(format!("{index_path}:{line}: 生命周期不在闭集内"));
                }
            }
            if let Some((value, line)) =
                metadata::text(input, parsed, &["current_version", "当前版本"])
                    .or_else(|| markdown::field(&lines, &["current_version", "当前版本"]))
                    .filter(|(value, _)| markdown::meaningful(value))
            {
                project.version = Sourced::known(value, markdown::location(&index_path, line));
            }
            inbox.extend(actions::index(&root, &name, &index_path, &lines));
        } else {
            project.diagnostics.push("缺少 00-总览/INDEX.md".to_owned());
        }
        if project.lifecycle.value.is_none() {
            project
                .diagnostics
                .push("生命周期未知：没有有效的明确声明".to_owned());
        }
        if project.version.value.is_none() {
            project
                .diagnostics
                .push("当前版本未知：没有有效的明确声明".to_owned());
        }
        if let Some((input, _)) = inputs.get(&tasks_path) {
            let lines = markdown::lines(input.contents());
            project.active_tasks = active_tasks(&tasks_path, &lines, &mut project.diagnostics);
            inbox.extend(actions::tasks(&root, &name, &tasks_path, &lines));
        } else {
            project
                .diagnostics
                .push("进行中任务数未知：缺少 04-协调/TASKS.md".to_owned());
        }
        if let Some((input, _)) = inputs.get(&roadmap_path) {
            project.next_milestone =
                next_milestone(&roadmap_path, input.contents(), &mut project.diagnostics);
        } else {
            project
                .diagnostics
                .push("下一里程碑未知：缺少 05-版本/roadmap.md".to_owned());
        }
        project.open_bugs =
            bugs::open_bugs(&root, &name, inputs, &mut project.diagnostics, &mut inbox);
        let memory_prefix = format!("{root}/03-记忆/");
        for (path, (input, parsed)) in scan::under(inputs, &memory_prefix) {
            if direct_child(path, &memory_prefix)
                && path
                    .rsplit('/')
                    .next()
                    .is_some_and(|file| file.starts_with("ADR-"))
                && let Some(action) = actions::frontmatter(&root, &name, input, parsed)
            {
                inbox.push(action);
            }
        }
        projects.push(project);
    }
    inbox.sort_by(|left, right| {
        (
            &left.project_id,
            &left.source.path,
            left.source.line,
            &left.kind,
        )
            .cmp(&(
                &right.project_id,
                &right.source.path,
                right.source.line,
                &right.kind,
            ))
    });
    inbox.dedup_by(|left, right| left.id == right.id);
    (projects, inbox)
}

fn active_tasks(
    path: &str,
    lines: &[markdown::Line<'_>],
    diagnostics: &mut Vec<String>,
) -> Sourced<u64> {
    let Some((_, section)) = markdown::section(lines, '①') else {
        diagnostics.push(format!("{path}: 进行中任务数未知：缺少 ① 区"));
        return Sourced::unknown();
    };
    let mut count = 0;
    let mut source = None;
    let mut invalid = false;
    for table in markdown::tables(&section) {
        let (Some(title), Some(status)) = (
            table.column(&["任务", "内容"]),
            table.column(&["状态", "status"]),
        ) else {
            continue;
        };
        source.get_or_insert_with(|| markdown::location(path, table.line));
        for row in table.rows {
            if row.cells.iter().all(|cell| !markdown::meaningful(cell)) {
                continue;
            }
            let Some(title) = row.cells.get(title) else {
                invalid = true;
                continue;
            };
            if !markdown::meaningful(title) {
                continue;
            }
            let Some(status) = row
                .cells
                .get(status)
                .filter(|value| markdown::meaningful(value))
            else {
                diagnostics.push(format!("{path}:{}: 任务状态缺失，不能可靠计数", row.line));
                invalid = true;
                continue;
            };
            if !markdown::completed(status) {
                count += 1;
            }
        }
    }
    if invalid {
        return Sourced::unknown();
    }
    source.map_or_else(
        || {
            diagnostics.push(format!("{path}: 进行中任务数未知：① 区没有合法任务表"));
            Sourced::unknown()
        },
        |source| Sourced::known(count, source),
    )
}

fn next_milestone(path: &str, content: &str, diagnostics: &mut Vec<String>) -> Sourced<String> {
    let lines = markdown::current_lines(&markdown::lines(content));
    for table in markdown::tables(&lines) {
        let Some(title_column) = table.column(&["里程碑", "milestone"]) else {
            continue;
        };
        let status_column = table.column(&["状态", "status"]);
        for row in table.rows {
            let Some(title) = row
                .cells
                .get(title_column)
                .filter(|text| markdown::meaningful(text))
            else {
                continue;
            };
            let status = status_column.and_then(|column| row.cells.get(column));
            if status.is_some_and(|text| markdown::completed(text)) {
                continue;
            }
            if status.is_none_or(|text| !markdown::meaningful(text)) {
                diagnostics.push(format!(
                    "{path}:{}: 里程碑完成状态未声明，保守保留为下一项",
                    row.line
                ));
            }
            return Sourced::known(title.clone(), markdown::location(path, row.line));
        }
    }
    diagnostics.push(format!("{path}: 没有可确认的下一里程碑"));
    Sourced::unknown()
}

pub(crate) fn direct_child(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|relative| !relative.contains('/') && relative.ends_with(".md"))
}
