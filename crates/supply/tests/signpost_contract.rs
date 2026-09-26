//! 路牌契约：六项齐全、全机器派生、缺失字段显示不可用、硬预算二元。

use std::fs;

use studiolo_core::VaultPath;
use studiolo_supply::{
    HARD_BUDGET_TOKENS, Principal, ScanLimits, SignpostInput, SupplyRoots, TARGET_BUDGET_TOKENS,
    count_tokens, signpost,
};
use tempfile::TempDir;

const TOOLS: &[(&str, &str)] = &[
    ("handshake", "重取路牌"),
    ("list_entries", "列白名单项目条目"),
    ("read_note", "读一篇笔记"),
    ("search", "全文或路径搜索"),
];

fn principal() -> Principal {
    Principal {
        agent_type: "claude".into(),
        instance_id: "01JT3STUDIOLO0000000000001".into(),
        registered_at: "2026-09-07T01:00:00Z".into(),
    }
}

fn write_project(root: &std::path::Path, name: &str, focus: &str, tasks: bool) {
    let base = root.join("项目").join(name);
    fs::create_dir_all(base.join("00-总览")).unwrap();
    fs::create_dir_all(base.join("02-日志")).unwrap();
    fs::write(
        base.join("00-总览/INDEX.md"),
        format!(
            "# {name} · 总览\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | v0.3 |\n| 状态 | 🟡 进行中 |\n| 最近日志 | [[2026-09-06]] |\n| 未关闭 BUG | 无 |\n\n## 当前重点\n\n- **{focus}**：说明文字。\n- 第二条不该出现。\n"
        ),
    )
    .unwrap();
    fs::write(base.join("02-日志/2026-09-05.md"), "# 旧日志\n").unwrap();
    fs::write(base.join("02-日志/2026-09-06.md"), "# 新日志\n").unwrap();
    if tasks {
        fs::create_dir_all(base.join("04-协调")).unwrap();
        fs::write(
            base.join("04-协调/TASKS.md"),
            "# 协调\n\n## ① 进行中\n\n| ID | 任务 | 负责线 | 文件 | 状态 | 日期 |\n|---|---|---|---|---|---|\n| T-033 | M3 首片 | Claude | crates/supply | 进行中：按计划 | 2026-09-06 |\n\n**待认领（T-031 复审登记，2026-09-06）**：修复项。\n\n## ② 占用\n\n| T-999 | 不该被当进行中 |\n",
        )
        .unwrap();
    }
}

fn write_rules(root: &std::path::Path) {
    fs::create_dir_all(root.join("_规则")).unwrap();
    fs::write(
        root.join("_规则/通用工作规则.md"),
        "# 规则\n\n# 第一部分 核心行为准则（无任何例外）\n\n## 准则① 永远说中文\n\n正文。\n\n## 准则② 最严谨修复\n\n正文。\n\n## 其他\n\n3. **不属于准则**\n",
    )
    .unwrap();
    fs::write(
        root.join("项目/Alpha/CLAUDE.md"),
        "# 全局规则\n\n## 核心行为准则（无例外）\n\n1. **快照里的准则**：不应覆盖母本。\n\n**只要在写 / 改 / 评审 React 代码**，一律叠加 `vercel-react-best-practices`。\n",
    )
    .unwrap();
}

#[test]
fn signpost_has_six_sections_and_is_derived_from_index_tasks_and_rules() {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    write_project(temp.path(), "Alpha", "T-033 冷启动", true);
    write_project(temp.path(), "Bare", "无协调板", false);
    write_rules(temp.path());
    let allow = ["项目/Alpha", "项目/Bare", "_规则"];
    let roots = SupplyRoots::open(temp.path(), &allow).unwrap();
    let snapshot = roots.scan(ScanLimits::default()).unwrap();
    let allow = allow.map(|value| VaultPath::parse(value).unwrap());
    let signpost = signpost::derive(&SignpostInput {
        snapshot: &snapshot,
        allow: &allow,
        principal: &principal(),
        generation: 7,
        generated_at: "2026-09-07T01:00:00Z",
        freshness: None,
        tools: TOOLS,
        policy: None,
        rules_whitelist: None,
    })
    .unwrap();
    let text = &signpost.text;
    for heading in [
        "## 1 我是谁",
        "## 2 项目状态",
        "## 3 能力",
        "## 4 待我做的事",
        "## 5 硬约束",
        "## 6 继续拉",
    ] {
        assert!(text.contains(heading), "missing {heading}:\n{text}");
    }
    assert!(text.contains("类型 claude · 实例 01JT3STUDIOLO0000000000001"));
    assert!(
        text.contains("- 项目/Alpha：🟡 进行中 · 未关闭 BUG 无 · 重点：T-033 冷启动：说明文字。")
    );
    assert!(!text.contains("第二条不该出现"));
    assert!(text.contains("进行中：项目/Alpha · T-033 M3 首片（进行中：按计划）"));
    assert!(text.contains("待认领：项目/Alpha · T-031 复审登记，2026-09-06"));
    assert!(!text.contains("T-999"));
    assert!(text.contains("1 永远说中文；2 最严谨修复"));
    assert!(!text.contains("不属于准则"));
    assert!(text.contains(
        "硬绑定：只要在写 / 改 / 评审 React 代码，一律叠加 vercel-react-best-practices。"
    ));
    assert!(
        !text.contains("快照里的准则"),
        "母本存在时不用 CLAUDE.md 快照的准则"
    );
    assert!(text.contains("入口：项目/Alpha/CLAUDE.md@"));
    assert!(text.contains("只读；白名单：项目/Alpha、项目/Bare、_规则；写与审批：无（M3）"));
    assert!(text.contains("tools：handshake（重取路牌）"));
    let index = snapshot
        .note(&VaultPath::parse("项目/Alpha/00-总览/INDEX.md").unwrap())
        .unwrap();
    assert!(text.contains(&format!(
        "入口：项目/Alpha/00-总览/INDEX.md@{}",
        &index.revision.to_string()[..12]
    )));
    assert!(text.contains("入口：项目/Alpha/02-日志/2026-09-06.md@"));
    assert!(!text.contains("2026-09-05.md"));
    assert!(text.contains("入口：_规则/通用工作规则.md@"));
    assert!(
        !text.contains("- 项目/Bare：不可用"),
        "Bare has an INDEX table"
    );
    assert!(text.contains("- 项目/Bare："));
    assert!(
        !text.contains("# 旧日志")
            && !text.contains("| 当前版本 |")
            && !text.contains("第二条不该出现"),
        "no bodies leak"
    );
    assert_eq!(signpost.trim_level, 0);
    assert!(signpost.tokens <= TARGET_BUDGET_TOKENS);
    assert_eq!(signpost.tokens, count_tokens(text));
}

#[test]
fn missing_skeleton_shows_unavailable_and_rules_absent_is_stated() {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    fs::create_dir_all(temp.path().join("项目/Odd/00-总览")).unwrap();
    fs::write(
        temp.path().join("项目/Odd/00-总览/INDEX.md"),
        "# 没有表格\n",
    )
    .unwrap();
    let roots = SupplyRoots::open(temp.path(), &["项目/Odd"]).unwrap();
    let snapshot = roots.scan(ScanLimits::default()).unwrap();
    let allow = [VaultPath::parse("项目/Odd").unwrap()];
    let text = signpost::derive(&SignpostInput {
        snapshot: &snapshot,
        allow: &allow,
        principal: &principal(),
        generation: 1,
        generated_at: "2026-09-07T01:00:00Z",
        freshness: None,
        tools: TOOLS,
        policy: None,
        rules_whitelist: None,
    })
    .unwrap()
    .text;
    // 骨架缺失时状态字段仍须可见为「不可用」；focus 缺失不再占一行（清单化后省 token）
    assert!(text.contains("：不可用 · 未关闭 BUG 不可用"));
    assert!(!text.contains("· 重点："));
    assert!(text.contains("无（协调板无进行中或待认领项，或不可用）"));
    assert!(text.contains("不可用（白名单内无规则母本）"));
}

/// 触发硬预算的夹具：清单化后每项目只贡献一行摘要，纯靠数量在三级裁剪下增长过慢，
/// 改用「多项目 ＋ 长项目名」——项目名同时出现在项目清单与能力段白名单里，且两处都不可裁剪。
const OVERSIZED_PROJECTS: usize = 180;
const OVERSIZED_NAME_PAD: usize = 150;

#[test]
fn oversized_material_is_trimmed_then_fails_closed_past_the_hard_budget() {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    // 清单化 + 预算上调到 8000 后，须显著更多项目才触到硬预算；本例验证的是「超出即失败关闭」本身。
    for index in 0..OVERSIZED_PROJECTS {
        let name = format!("P{index}{}", "x".repeat(OVERSIZED_NAME_PAD));
        write_project(temp.path(), &name, &"重点".repeat(100), true);
        let tasks = temp
            .path()
            .join("项目")
            .join(&name)
            .join("04-协调/TASKS.md");
        let body = fs::read_to_string(&tasks).unwrap();
        let rows = (0..40)
            .map(|row| {
                format!(
                    "| T-{row:03} | {} | Claude | crates | 进行中 | 2026-09-06 |\n",
                    "很长的任务描述".repeat(12)
                )
            })
            .collect::<String>();
        fs::write(
            tasks,
            body.replace("\n**待认领", &format!("{rows}\n**待认领")),
        )
        .unwrap();
    }
    let allow = (0..OVERSIZED_PROJECTS)
        .map(|index| format!("项目/P{index}{}", "x".repeat(OVERSIZED_NAME_PAD)))
        .collect::<Vec<_>>();
    let allow_refs = allow.iter().map(String::as_str).collect::<Vec<_>>();
    let roots = SupplyRoots::open(temp.path(), &allow_refs).unwrap();
    let snapshot = roots.scan(ScanLimits::default()).unwrap();
    let allow = allow
        .iter()
        .map(|value| VaultPath::parse(value).unwrap())
        .collect::<Vec<_>>();
    let input = SignpostInput {
        snapshot: &snapshot,
        allow: &allow,
        principal: &principal(),
        generation: 1,
        generated_at: "2026-09-07T01:00:00Z",
        freshness: None,
        tools: TOOLS,
        policy: None,
        rules_whitelist: None,
    };
    let error = signpost::derive(&input).unwrap_err();
    assert!(error.tokens > HARD_BUDGET_TOKENS);
    assert_eq!(error.limit, HARD_BUDGET_TOKENS);
}

#[test]
fn freshness_is_never_trimmed_out_of_the_hard_handshake_budget() {
    let snapshot = studiolo_supply::Snapshot::default();
    let principal = principal();
    let freshness = "required replica freshness ".repeat(3000);
    let error = signpost::derive(&SignpostInput {
        snapshot: &snapshot,
        allow: &[],
        principal: &principal,
        generation: 1,
        generated_at: "2026-09-09T00:00:00Z",
        freshness: Some(&freshness),
        tools: TOOLS,
        policy: None,
        rules_whitelist: None,
    })
    .unwrap_err();
    assert!(error.tokens > HARD_BUDGET_TOKENS);
}
