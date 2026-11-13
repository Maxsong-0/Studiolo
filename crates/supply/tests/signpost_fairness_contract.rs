//! 路牌第 4 项按项目轮流取（T-193）：项目一多时，不能只让字典序靠前的两三个项目占满条数。

use std::{collections::BTreeSet, fs, path::Path};

use studiolo_core::VaultPath;
use studiolo_supply::{Principal, ScanLimits, Signpost, SignpostInput, SupplyRoots, signpost};
use tempfile::TempDir;

const TOOLS: &[(&str, &str)] = &[("handshake", "重取路牌")];

/// 一个夹具项目：名字、进行中条数、待认领条数。
struct Project {
    name: String,
    in_progress: usize,
    unclaimed: usize,
}

fn uniform(count: usize, in_progress: usize, unclaimed: usize) -> Vec<Project> {
    (0..count)
        .map(|index| Project {
            name: format!("P{index:02}"),
            in_progress,
            unclaimed,
        })
        .collect()
}

fn write_project(root: &Path, project: &Project) {
    let name = &project.name;
    let base = root.join("项目").join(name);
    fs::create_dir_all(base.join("00-总览")).unwrap();
    fs::create_dir_all(base.join("04-协调")).unwrap();
    fs::write(
        base.join("00-总览/INDEX.md"),
        format!(
            "# {name} · 总览\n\n| 项 | 当前值 |\n|---|---|\n| 状态 | 🟡 进行中 |\n| 未关闭 BUG | 无 |\n"
        ),
    )
    .unwrap();
    let rows = (1..=project.in_progress)
        .map(|row| {
            format!("| T-{row:03} | {name} 第 {row} 条 | Claude | crates | 进行中 | 2026-11-12 |\n")
        })
        .collect::<String>();
    let titles = (1..=project.unclaimed)
        .map(|row| format!("\n**待认领（{name} 待认领 {row}）**：说明。\n"))
        .collect::<String>();
    fs::write(
        base.join("04-协调/TASKS.md"),
        format!(
            "# 协调\n\n## ① 进行中\n\n| ID | 任务 | 负责线 | 文件 | 状态 | 日期 |\n|---|---|---|---|---|---|\n{rows}{titles}"
        ),
    )
    .unwrap();
}

/// 只经公开接口 `signpost::derive` 派生，所以在改动前的实现上同样能编译。
fn derive(projects: &[Project]) -> Signpost {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    for project in projects {
        write_project(temp.path(), project);
    }
    let allow = projects
        .iter()
        .map(|project| format!("项目/{}", project.name))
        .collect::<Vec<_>>();
    let allow_refs = allow.iter().map(String::as_str).collect::<Vec<_>>();
    let roots = SupplyRoots::open(temp.path(), &allow_refs).unwrap();
    let snapshot = roots.scan(ScanLimits::default()).unwrap();
    let allow = allow
        .iter()
        .map(|value| VaultPath::parse(value).unwrap())
        .collect::<Vec<_>>();
    let principal = Principal {
        agent_type: "claude".into(),
        instance_id: "01JT3STUDIOLO0000000000001".into(),
        registered_at: "2026-11-12T01:00:00Z".into(),
    };
    signpost::derive(&SignpostInput {
        snapshot: &snapshot,
        allow: &allow,
        principal: &principal,
        generation: 1,
        generated_at: "2026-11-12T01:00:00Z",
        freshness: None,
        tools: TOOLS,
        policy: None,
        rules_whitelist: None,
    })
    .unwrap()
}

/// 某一节的全部行（不含标题行）。
fn section<'a>(text: &'a str, heading: &str, next: &str) -> Vec<&'a str> {
    let start = text.find(heading).unwrap() + heading.len();
    let end = start + text[start..].find(next).unwrap();
    text[start..end]
        .lines()
        .filter(|line| !line.is_empty())
        .collect()
}

/// 第 4 项里以 `label` 开头的行，取「：」与「 · 」之间的项目前缀。
fn listed_projects<'a>(todo: &[&'a str], label: &str) -> Vec<&'a str> {
    todo.iter()
        .copied()
        .filter_map(|line| line.strip_prefix(label))
        .map(|rest| rest.split(" · ").next().unwrap())
        .collect()
}

#[test]
fn in_progress_rows_come_from_as_many_projects_as_the_limit_allows() {
    let signpost = derive(&uniform(10, 3, 0));
    assert_eq!(signpost.trim_level, 0, "{}", signpost.text);
    let todo = section(&signpost.text, "## 4 待我做的事\n", "## 5 硬约束");
    let listed = listed_projects(&todo, "进行中：");
    assert_eq!(listed.len(), 8, "{todo:#?}");
    assert_eq!(
        listed.iter().collect::<BTreeSet<_>>().len(),
        8,
        "8 条进行中应来自 8 个不同项目：{todo:#?}"
    );
}

#[test]
fn rounds_take_first_rows_in_project_order_and_the_footer_states_totals() {
    let signpost = derive(&uniform(10, 3, 0));
    let text = &signpost.text;
    let todo = section(text, "## 4 待我做的事\n", "## 5 硬约束");
    let expected = (0..8)
        .map(|index| format!("进行中：项目/P{index:02} · T-001 P{index:02} 第 1 条（进行中）"))
        .collect::<Vec<_>>();
    assert_eq!(todo[..8], expected, "{todo:#?}");
    assert!(text.contains("共 30 条进行中") && text.contains("这里列出 8＋"));
    assert_eq!(
        todo[8],
        "（共 30 条进行中、0 条待认领，这里列出 8＋0 条；各项目条数见第 2 节，完整见各项目 04-协调/TASKS.md）"
    );
    assert_eq!(todo.len(), 9, "{todo:#?}");
    let status = section(text, "## 2 项目状态\n", "## 3 能力");
    let rows = status
        .iter()
        .filter(|line| line.starts_with("- 项目/"))
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 10, "{status:#?}");
    for row in rows {
        assert!(row.contains("进行中 3／待认领 0"), "{row}");
        assert!(
            row.ends_with(" · 未关闭 BUG 无 · 进行中 3／待认领 0"),
            "{row}"
        );
    }
}

#[test]
fn unclaimed_titles_also_rotate_across_projects() {
    let signpost = derive(&uniform(4, 0, 2));
    assert_eq!(signpost.trim_level, 0, "{}", signpost.text);
    let todo = section(&signpost.text, "## 4 待我做的事\n", "## 5 硬约束");
    assert_eq!(
        listed_projects(&todo, "待认领："),
        ["项目/P00", "项目/P01", "项目/P02"],
        "{todo:#?}"
    );
    assert!(listed_projects(&todo, "进行中：").is_empty());
    assert!(signpost.text.contains(
        "（共 0 条进行中、8 条待认领，这里列出 0＋3 条；各项目条数见第 2 节，完整见各项目 04-协调/TASKS.md）"
    ));
    assert!(signpost.text.contains("进行中 0／待认领 2"));
}

#[test]
fn a_single_project_keeps_the_original_row_order() {
    let signpost = derive(&[Project {
        name: "Solo".into(),
        in_progress: 10,
        unclaimed: 0,
    }]);
    let todo = section(&signpost.text, "## 4 待我做的事\n", "## 5 硬约束");
    let expected = (1..=8)
        .map(|row| format!("进行中：项目/Solo · T-{row:03} Solo 第 {row} 条（进行中）"))
        .collect::<Vec<_>>();
    assert_eq!(todo[..8], expected, "{todo:#?}");
    assert_eq!(listed_projects(&todo, "进行中：").len(), 8);
}
