//! 能力集规则名单 `read.rules_whitelist`（ADR-026 D3／D4）：旧能力集缺字段即零规则；每项须是合法且
//! 不重复的规则 subject，否则按下标报问题；相对现行版本，新增项报扩大、移除项报收窄。

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use serde_json::{Value, json};
use studiolo_policy::{
    CapabilitySet, DesiredInput, DesiredRecord, DesiredStatus, Expansion, PolicyKind, PolicyStore,
    Submitter,
};
use tempfile::TempDir;

const NOW: &str = "2026-09-26T22:00:00Z";

fn private_dir() -> (TempDir, PathBuf) {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let dir = temp.path().join("policy-data");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    (temp, dir)
}

/// `rules` 为 `None` 时正文里根本没有 `rules_whitelist` 字段（旧能力集的形状）。
fn capability(rules: Option<&[&str]>) -> Value {
    let mut read =
        json!({"project_whitelist": ["项目/Studiolo", "_规则"], "deny_by_default": true});
    if let Some(rules) = rules {
        read["rules_whitelist"] = json!(rules);
    }
    json!({
        "ai_type": "claude",
        "read": read,
        "write": {"board": false, "content_patch_only": true, "content_direct": false},
        "policy": {"propose_desired": false, "activate": false},
        "hard_bound_skills": [],
    })
}

fn submit(store: &PolicyStore, body: &Value) -> DesiredRecord {
    store
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::CapabilitySet,
                subject: "claude".into(),
                body: Some(body.to_string()),
            },
            Submitter::Owner,
            NOW,
        )
        .unwrap()
}

fn publish(store: &PolicyStore, body: &Value) -> DesiredRecord {
    let record = submit(store, body);
    assert_eq!(
        record.status,
        DesiredStatus::Validated,
        "{:?}",
        record.report
    );
    let hash = store.effective().unwrap().effective_hash;
    store.activate(&record.id, &hash, "owner", NOW).unwrap();
    record
}

/// 报告里 `read.rules_whitelist` 的扩大项或收窄项。
fn rules_items(items: &[Expansion]) -> Vec<(Option<&str>, Option<&str>, &str)> {
    items
        .iter()
        .filter(|item| item.path == "read.rules_whitelist")
        .map(|item| {
            (
                item.before.as_deref(),
                item.after.as_deref(),
                item.note.as_str(),
            )
        })
        .collect()
}

#[test]
fn a_capability_set_without_the_field_reads_no_rules() {
    let legacy: CapabilitySet = serde_json::from_value(capability(None)).unwrap();
    assert!(legacy.read.rules_whitelist.is_empty());
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let record = publish(&store, &capability(None));
    let report = record.report.as_ref().unwrap();
    assert!(rules_items(&report.expansions).is_empty(), "{report:?}");
    let (set, meta) = store.capability_set("claude").unwrap().unwrap();
    assert!(set.read.rules_whitelist.is_empty());
    // 规范化正文显式写出空名单：以后读到的都是同一形状，零规则一目了然。
    let body = store.version(meta.version).unwrap().unwrap().body.unwrap();
    assert!(body.contains("\"rules_whitelist\":[]"), "{body}");
}

#[test]
fn valid_rule_subjects_pass_and_first_grant_is_an_expansion() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let record = publish(&store, &capability(Some(&["通用工作规则.md", "CLAUDE.md"])));
    let report = record.report.as_ref().unwrap();
    assert!(report.ok && report.problems.is_empty(), "{report:?}");
    assert_eq!(
        rules_items(&report.expansions),
        [
            (None, Some("通用工作规则.md"), "新类型首次获得规则读取"),
            (None, Some("CLAUDE.md"), "新类型首次获得规则读取"),
        ]
    );
    let (set, _) = store.capability_set("claude").unwrap().unwrap();
    assert_eq!(set.read.rules_whitelist, ["通用工作规则.md", "CLAUDE.md"]);
}

#[test]
fn invalid_or_duplicate_rule_subjects_are_rejected_at_their_index() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    let record = submit(
        &store,
        &capability(Some(&[
            "../x.md",
            "a/b.md",
            "x.txt",
            ".x.md",
            "CLAUDE.md",
            "CLAUDE.md",
        ])),
    );
    assert_eq!(record.status, DesiredStatus::Rejected);
    let report = record.report.as_ref().unwrap();
    let problems = report
        .problems
        .iter()
        .map(|problem| (problem.path.as_str(), problem.message.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        problems,
        [
            ("read.rules_whitelist[0]", "不是合法的规则 subject"),
            ("read.rules_whitelist[1]", "不是合法的规则 subject"),
            ("read.rules_whitelist[2]", "不是合法的规则 subject"),
            ("read.rules_whitelist[3]", "不是合法的规则 subject"),
            ("read.rules_whitelist[5]", "重复的规则 subject"),
        ]
    );
    assert!(store.capability_set("claude").unwrap().is_none());
}

#[test]
fn adding_a_rule_is_an_expansion_and_removing_one_is_a_narrowing() {
    let (_temp, dir) = private_dir();
    let store = PolicyStore::open(&dir).unwrap();
    publish(&store, &capability(Some(&["A.md"])));
    let swapped = submit(&store, &capability(Some(&["B.md"])));
    assert_eq!(swapped.status, DesiredStatus::Validated);
    let report = swapped.report.as_ref().unwrap();
    assert_eq!(
        rules_items(&report.expansions),
        [(None, Some("B.md"), "规则读取范围变宽")]
    );
    assert_eq!(
        rules_items(&report.narrowings),
        [(Some("A.md"), None, "规则读取范围收窄")]
    );
    // 名单不变不报任何一项；删掉整个字段等于收回全部规则。
    let same = submit(&store, &capability(Some(&["A.md"])));
    let report = same.report.as_ref().unwrap();
    assert!(rules_items(&report.expansions).is_empty());
    assert!(rules_items(&report.narrowings).is_empty());
    let dropped = submit(&store, &capability(None));
    let report = dropped.report.as_ref().unwrap();
    assert!(rules_items(&report.expansions).is_empty());
    assert_eq!(
        rules_items(&report.narrowings),
        [(Some("A.md"), None, "规则读取范围收窄")]
    );
}
