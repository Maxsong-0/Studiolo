//! 路牌的可见段（T-263，vault BUG-061）：Claude Code 只把 MCP `instructions` 的前 4096 个 UTF-16 单元交给模型，
//! 后面的换成「… [truncated]」。硬约束、续读入口、能力、待办和第 2 节开头（容量提醒、项目数）必须在线内，
//! 线外只许剩项目清单；线内放不下时只裁可见段自己，不动项目清单。
//!
//! 白名单照生产单元模板里的 `--allow`（和项目登记表一致由 `test-project-registry.py` 守着），工具说明用服务端真正发的
//! `TOOLS`：生产加项目、工具加说明，这里跟着变长，压线时先红。

use std::{collections::BTreeSet, fs, path::Path};

use studiolo_agent_server::server::TOOLS;
use studiolo_core::VaultPath;
use studiolo_supply::{
    PolicyRule, PolicySignpost, Principal, ScanLimits, Signpost, SignpostInput, SupplyRoots,
    count_tokens, signpost,
};
use tempfile::TempDir;

/// 客户端的上限，照官方文档和 2026-11-28 对 2.1.332 的实测写死，不引产品常量：产品常量改错了这里要红。
const CLIENT_LIMIT_UTF16: usize = 4096;
/// 生产新鲜度机器行的长度和形状（`replica::Freshness::describe`），托管模式自带一个换行。
const FRESHNESS: &str = "副本状态 ready；stale=false；generation=01JT3STUDIOLO0000000000002；source_digest=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa；source_observed_at_ms=1795894200000；received_at_ms=1795894203000；age_secs=42；max_age_secs=300\n";
const UNIT: &str = include_str!("../../../deploy/agent/studiolo-agent-system.service.in");
/// 第 5 节最后一条准则：它在线内，前面的都在。
const LAST_PRINCIPLE: &str = "只管本会话所在的项目";
const PRINCIPLES: [&str; 11] = [
    "永远说中文",
    "根因修复，同类修全",
    "先懂需求再动手",
    "灵活用工具链",
    "纠正只发生一次",
    "并发协作意识",
    "范围外的问题上报不擅改",
    "代码要好理解、好测试、好修改",
    "网上的数据优先交给 GrokBot",
    "灵活派活给外部代理",
    LAST_PRINCIPLE,
];

/// 四条上限都到线的容量提醒（`replica::capacity_notice_for`），最长的一种。
const CAPACITY: &str = "⚠ 副本快到协议上限：Markdown 合计 33.0 MiB，上限 40.0 MiB 的 82%；文件总量 420.0 MiB，上限 512.0 MiB 的 82%；可见条目 20500，上限 25000 的 82%；清单 3.3 MiB，上限 4.0 MiB 的 82%。涨过任一条上限后 Mac 同步会整批失败、这里的资料停在越线前（vault BUG-039）。请提醒用户趁早调高上限（两端常量，见 docs/m3/replica-freshness.md）或精简白名单。";

/// 生产白名单：单元模板里逐个 `--allow "<路径>"`。
fn production() -> Vec<String> {
    UNIT.split("--allow \"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap().to_owned())
        .collect()
}

/// 白名单再多几个没有总览的目录（同 `项目/HR系统-Docs`）：只撑长第 3 节，不撑项目清单。
fn with_bare_directories(extra: usize) -> Vec<String> {
    let mut allow = production();
    allow.extend((0..extra).map(|index| format!("项目/没有总览的目录{index:02}-Docs")));
    allow
}

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

/// 一份待办：每个项目 5 条进行中、3 条待认领，任务和标题都用给定文字。
fn tasks(task: &str) -> String {
    let rows = (1..=5)
        .map(|row| format!("| T-{row:03} | {task} | Claude | crates | 进行中 | 2026-11-28 |\n"))
        .collect::<String>();
    let titles = (1..=3)
        .map(|row| format!("\n**待认领（第 {row} 件 {task}）**：说明。\n"))
        .collect::<String>();
    format!(
        "# 协调\n\n## ① 进行中\n\n| ID | 任务 | 负责线 | 文件 | 状态 | 日期 |\n|---|---|---|---|---|---|\n{rows}{titles}"
    )
}

/// 没有子项时的黑板行（`server/board.rs`），服务端就这样插进路牌。
const BOARD: &str = "黑板实时状态：本实例持有 0 项。我派出的子项：0 项\n";

/// 按给定白名单、待办文字和容量提醒派生一份策略平面已接入的路牌，带黑板行。
fn derive(allow: &[String], task: &str, capacity: Option<&str>) -> Signpost {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    for path in allow {
        let base = temp.path().join(path);
        if !path.starts_with("项目/") {
            write(&base.join("通用工作规则.md"), "# 规则\n");
            continue;
        }
        if path.contains("没有总览") {
            write(&base.join("说明.md"), "# 说明\n");
            continue;
        }
        write(
            &base.join("00-总览/INDEX.md"),
            "# 总览\n\n| 项 | 当前值 |\n|---|---|\n| 状态 | 🟡 进行中：整体完成八成五，自用生产还差一半，下一步等验收 |\n| 未关闭 BUG | [[BUG-051-写者正文记录满额]] 修复中，等上线 |\n\n## 当前重点\n\n- **这一阶段在做的事**：说明文字写得比较长，长到会被截成六十个字，看看项目清单那一行还剩多少内容没有被裁掉。\n",
        );
        write(&base.join("04-协调/TASKS.md"), &tasks(task));
        write(&base.join("02-日志/2026-11-28.md"), "# 日志\n");
        write(&base.join("CLAUDE.md"), "# 全局规则在哪\n");
    }
    let refs = allow.iter().map(String::as_str).collect::<Vec<_>>();
    let roots = SupplyRoots::open(temp.path(), &refs).unwrap();
    let snapshot = roots.scan(ScanLimits::default()).unwrap();
    let prefixes = allow
        .iter()
        .map(|value| VaultPath::parse(value.as_str()).unwrap())
        .collect::<Vec<_>>();
    let principal = Principal {
        agent_type: "claude".into(),
        instance_id: "01JT3STUDIOLO0000000000001".into(),
        registered_at: "2026-11-28T19:30:00.000000000Z".into(),
    };
    let body = PRINCIPLES
        .iter()
        .map(|title| format!("## 准则 {title}\n\n正文。\n\n"))
        .collect::<String>();
    let policy = PolicySignpost {
        board_write: true,
        effective_hash: "0123456789abcdef".repeat(4),
        capability_version: 25,
        read_prefixes: prefixes.clone(),
        content_patch_only: true,
        propose_desired: true,
        hard_bindings: vec![(
            "写或改 React / Next.js 代码".into(),
            "vercel-react-best-practices".into(),
        )],
        rules: vec![
            PolicyRule {
                subject: "CLAUDE.md".into(),
                version: 23,
                body: String::new(),
            },
            PolicyRule {
                subject: "通用工作规则.md".into(),
                version: 22,
                body,
            },
        ],
        skills: vec!["vercel-react-best-practices".into(); 9],
    };
    let rules = ["CLAUDE.md".to_owned(), "通用工作规则.md".to_owned()];
    let mut signpost = signpost::derive(&SignpostInput {
        snapshot: &snapshot,
        allow: &prefixes,
        principal: &principal,
        generation: 123,
        generated_at: "2026-11-28T19:30:00.000000000Z",
        freshness: Some(FRESHNESS),
        capacity,
        tools: TOOLS,
        policy: Some(&policy),
        rules_whitelist: Some(&rules),
    })
    .unwrap();
    signpost.insert_board(BOARD);
    assert_eq!(signpost.tokens, count_tokens(&signpost.text));
    signpost
}

/// 客户端交给模型的那一段：前 4096 个 UTF-16 单元，emoji 占两个。
fn visible(text: &str) -> &str {
    let mut units = 0;
    let end = text
        .char_indices()
        .find(|(_, character)| {
            units += character.len_utf16();
            units > CLIENT_LIMIT_UTF16
        })
        .map_or(text.len(), |(at, _)| at);
    &text[..end]
}

/// 硬约束与续读入口：任何规模、任何裁剪等级都必须整节在线内。
fn assert_rules_and_entries_are_visible(text: &str) {
    let seen = visible(text);
    for needed in [
        "## 5 硬约束\n",
        LAST_PRINCIPLE,
        "硬绑定（策略强制）：写或改 React / Next.js 代码 → vercel-react-best-practices\n",
        "## 6 继续拉\n",
        "入口规律：任一可读项目的 <项目前缀>/00-总览/INDEX.md",
        "入口：policy://rules/通用工作规则.md@v22\n",
        "skills：9 个（policy_read kind=skills 取索引）\n",
        "调 handshake 取全文",
    ] {
        assert!(seen.contains(needed), "线内没有 {needed:?}：\n{seen}");
    }
    let tools_line = seen
        .lines()
        .find(|line| line.starts_with("tools："))
        .expect("线内没有工具清单");
    for (name, _) in TOOLS {
        assert!(tools_line.contains(name), "工具清单缺 {name}：{tools_line}");
    }
}

/// 整个可见段：再加能力、待办全部各行、第 2 节开头两行。
fn assert_front_is_visible(text: &str) {
    assert!(
        text.encode_utf16().count() > CLIENT_LIMIT_UTF16,
        "夹具没有长到会被截，测不出东西：\n{text}"
    );
    assert_rules_and_entries_are_visible(text);
    let seen = visible(text);
    for needed in [
        "；策略版本 0123456789ab\n",
        "## 4 待我做的事\n黑板实时状态：",
        "完整见各项目 04-协调/TASKS.md）\n",
        "## 2 项目状态\n",
        // 可见段的最后一行，核到行尾：只核开头会漏掉被截掉半行的。
        "，详情读 <项目前缀>/00-总览/INDEX.md\n",
    ] {
        assert!(seen.contains(needed), "线内没有 {needed:?}：\n{seen}");
    }
    let listed = |part: &str| {
        part.lines()
            .filter(|line| line.starts_with("进行中：") || line.starts_with("待认领："))
            .count()
    };
    assert_eq!(listed(seen), listed(text), "有待办行被截：\n{seen}");
}

const SHORT_TASK: &str = "修同步全链路路径预算低于真实库、每轮观察失败（计划第三段）";

fn long_task() -> String {
    "很长的任务描述一直写到被截为止，".repeat(20)
}

#[test]
fn sections_are_ordered_by_what_a_truncated_reader_needs_first() {
    let text = derive(&production(), SHORT_TASK, None).text;
    let at = [
        "## 1 我是谁\n",
        "## 5 硬约束\n",
        "## 6 继续拉\n",
        "## 3 能力\n",
        "## 4 待我做的事\n",
        "## 2 项目状态\n",
    ]
    .map(|heading| {
        assert_eq!(text.matches(heading).count(), 1, "{heading:?}：\n{text}");
        text.find(heading).unwrap()
    });
    assert!(at.is_sorted(), "{at:?}：\n{text}");
}

#[test]
fn at_production_scale_everything_but_the_project_list_is_visible() {
    let allow = production();
    assert!(allow.len() >= 20, "单元模板读少了：{allow:?}");
    let signpost = derive(&allow, SHORT_TASK, None);
    assert_front_is_visible(&signpost.text);
    // 平常长度的待办不触发任何裁剪：工具说明、项目重点都在。
    assert_eq!(
        signpost.trim_level, 0,
        "平常长度的待办已经要裁剪：可见段快满了，先瘦身再加内容（docs/contracts/agent-supply.md 第 3 节）：\n{}",
        signpost.text
    );
    assert!(
        signpost.text.contains("handshake（重取冷启动路牌）"),
        "{}",
        signpost.text
    );
    assert!(signpost.text.contains(" · 重点："));
}

#[test]
fn long_rows_shorten_the_visible_part_and_leave_the_project_list_alone() {
    let signpost = derive(&with_bare_directories(20), &long_task(), None);
    assert_front_is_visible(&signpost.text);
    assert!(signpost.trim_level > 0, "夹具没有触发可见段裁剪");
    assert!(
        signpost.text.contains(" · 重点："),
        "为可见段裁剪时动了项目清单：\n{}",
        signpost.text
    );
}

#[test]
fn every_whitelist_size_keeps_the_front_visible_with_the_board_line_in_place() {
    // 待办写到最长，白名单一个一个加目录（每个约 18 个单元）：可见段连续扫过上限，裁深一级再扫。
    // 黑板行（派生之后才插）的余量、第 2 节开头两行、待办条数，少算哪一样都在这里红。
    for capacity in [None, Some(CAPACITY)] {
        let mut levels = BTreeSet::new();
        for extra in 0..=85 {
            let signpost = derive(&with_bare_directories(extra), &long_task(), capacity);
            assert_front_is_visible(&signpost.text);
            levels.insert(signpost.trim_level);
        }
        assert!(
            levels.len() >= 2,
            "没有扫过换级的那一处：{levels:?}（容量提醒 {}）",
            capacity.is_some()
        );
    }
}

#[test]
fn surrogate_pairs_count_twice_like_the_client_counts_them() {
    // 每条 120 字里一半是 emoji：按字符数放得下、按 UTF-16 单元放不下的那一段。
    let task = "🟡修".repeat(60);
    let signpost = derive(&with_bare_directories(20), &task, None);
    assert_front_is_visible(&signpost.text);
    assert!(signpost.trim_level > 0, "夹具没有触发可见段裁剪");
}

#[test]
fn the_capacity_notice_stays_visible_even_when_all_four_limits_are_near() {
    for (allow, task) in [
        (production(), SHORT_TASK.to_owned()),
        (with_bare_directories(20), long_task()),
    ] {
        let text = derive(&allow, &task, Some(CAPACITY)).text;
        assert_front_is_visible(&text);
        assert!(
            visible(&text).contains(&format!("## 2 项目状态\n{CAPACITY}\n")),
            "容量提醒被截：\n{text}"
        );
    }
}

#[test]
fn rules_and_entries_stay_visible_when_the_whitelist_outgrows_the_window() {
    // 白名单涨到能力段自己都放不下：握手照样成功，硬约束和续读入口仍整节在线内。
    let mut allow = production();
    allow.extend((0..150).map(|index| format!("项目/以后新加的项目{index:03}-Docs")));
    let text = derive(&allow, &long_task(), None).text;
    assert_rules_and_entries_are_visible(&text);
    assert!(!visible(&text).contains("## 2 项目状态"), "夹具不够大");
}

#[test]
fn project_claude_files_are_covered_by_the_entry_pattern_instead_of_one_line_each() {
    let text = derive(&production(), SHORT_TASK, None).text;
    assert!(
        !text
            .lines()
            .any(|line| line.starts_with("入口：项目/") && line.contains("/CLAUDE.md@")),
        "{text}"
    );
    let pattern = text
        .lines()
        .find(|line| line.starts_with("入口规律："))
        .unwrap();
    assert!(pattern.contains("CLAUDE.md"), "{pattern}");
    // 只有一个可读项目时没有「入口规律」，逐条入口照旧给。
    let single = derive(
        &["_规则".to_owned(), "项目/Studiolo-Docs".to_owned()],
        SHORT_TASK,
        None,
    )
    .text;
    assert!(!single.contains("入口规律："), "{single}");
    assert!(
        single.contains("入口：项目/Studiolo-Docs/CLAUDE.md@"),
        "{single}"
    );
}
