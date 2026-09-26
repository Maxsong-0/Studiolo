//! 规则文件判定（ADR-026 D9）与路牌库文件兜底按规则名单裁决（D5／D10／D11）：只有直接位于
//! `_规则/` 下的 `.md` 才是规则文件；名单外的规则文件既不提供准则、硬绑定，也不进入口，名单内照常；
//! 项目里的 `CLAUDE.md` 是项目文件，不受规则名单影响；`None`（未接策略平面）不过滤。

use std::{fs, path::Path};

use studiolo_core::VaultPath;
use studiolo_supply::{
    Principal, RULES_ROOT, ScanLimits, SignpostInput, SupplyRoots, rule_subject, signpost,
};
use tempfile::TempDir;

const TOOLS: &[(&str, &str)] = &[("handshake", "重取路牌"), ("read_note", "读一篇笔记")];

fn write(root: &Path, path: &str, body: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn only_markdown_files_directly_under_the_rules_root_are_rule_files() {
    assert_eq!(RULES_ROOT, "_规则");
    assert_eq!(rule_subject("_规则/x.md"), Some("x.md"));
    assert_eq!(
        rule_subject("_规则/通用工作规则.md"),
        Some("通用工作规则.md")
    );
    for path in [
        "_规则/sub/x.md",
        "_规则/references/x.md",
        "_规则/skills/demo/SKILL.md",
        "_规则/x.txt",
        "项目/_规则/x.md",
        "项目/Studiolo/CLAUDE.md",
        "_规则x/a.md",
        "_规则/",
        "_规则",
    ] {
        assert_eq!(rule_subject(path), None, "{path}");
    }
}

/// 同一份快照在给定规则名单下派生的路牌文本。
fn signpost_text(root: &Path, rules: Option<&[String]>) -> String {
    let allow = ["项目/Alpha", "_规则"];
    let snapshot = SupplyRoots::open(root, &allow)
        .unwrap()
        .scan(ScanLimits::default())
        .unwrap();
    let allow = allow.map(|value| VaultPath::parse(value).unwrap());
    let principal = Principal {
        agent_type: "claude".into(),
        instance_id: "01JT3STUDIOLO0000000000001".into(),
        registered_at: "2026-09-26T01:00:00Z".into(),
    };
    signpost::derive(&SignpostInput {
        snapshot: &snapshot,
        allow: &allow,
        principal: &principal,
        generation: 1,
        generated_at: "2026-09-26T01:00:00Z",
        freshness: None,
        tools: TOOLS,
        policy: None,
        rules_whitelist: rules,
    })
    .unwrap()
    .text
}

#[test]
fn signpost_rule_files_follow_the_rules_whitelist() {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let root = temp.path();
    write(
        root,
        "项目/Alpha/00-总览/INDEX.md",
        "# Alpha\n\n| 项 | 当前值 |\n|---|---|\n| 状态 | 进行中 |\n",
    );
    write(
        root,
        "项目/Alpha/CLAUDE.md",
        "# 项目规则\n\n## 核心行为准则\n\n1. **项目快照准则**：说明。\n",
    );
    write(
        root,
        "_规则/通用工作规则.md",
        "# 规则\n\n## 准则① 母本准则\n\n**写 React 一律叠加 `mother-skill`。**\n",
    );
    write(
        root,
        "_规则/CLAUDE.md",
        "# 规则快照\n\n## 核心行为准则\n\n1. **规则快照准则**：说明。\n",
    );
    let mother = ["入口：_规则/通用工作规则.md@", "1 母本准则", "mother-skill"];
    let snapshot_entry = "入口：_规则/CLAUDE.md@";
    let project_entry = "入口：项目/Alpha/CLAUDE.md@";

    // 未接策略平面：不过滤，母本优先提供准则。
    let text = signpost_text(root, None);
    assert!(mother.iter().all(|item| text.contains(item)), "{text}");
    assert!(text.contains(snapshot_entry) && text.contains(project_entry));

    // 名单里只有 CLAUDE.md：母本的准则、硬绑定与入口全部消失，准则退回项目快照（项目文件不受名单影响）。
    let text = signpost_text(root, Some(&["CLAUDE.md".to_owned()]));
    assert!(!mother.iter().any(|item| text.contains(item)), "{text}");
    assert!(text.contains("1 项目快照准则"), "{text}");
    assert!(text.contains(snapshot_entry) && text.contains(project_entry));

    // 名单里只有母本：母本照常，`_规则/CLAUDE.md` 的入口消失。
    let text = signpost_text(root, Some(&["通用工作规则.md".to_owned()]));
    assert!(mother.iter().all(|item| text.contains(item)), "{text}");
    assert!(!text.contains(snapshot_entry) && text.contains(project_entry));

    // 空名单：`_规则/` 下的规则文件一个不剩，项目里的 CLAUDE.md 仍在（D11）。
    let text = signpost_text(root, Some(&[]));
    assert!(!mother.iter().any(|item| text.contains(item)), "{text}");
    assert!(!text.contains(snapshot_entry) && text.contains(project_entry));
    assert!(!text.contains("规则快照准则"));
}
