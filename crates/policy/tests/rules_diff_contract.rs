//! 规则审阅必须扫描完整变更，展示截断与 LCS 预算都不能隐藏约束删除。

use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_policy::{DesiredInput, PolicyKind, PolicyStore, Report, Submitter};

const NOW: &str = "2026-09-29T00:00:00Z";

fn input(body: &str) -> DesiredInput {
    DesiredInput {
        kind: PolicyKind::Rules,
        subject: "通用工作规则.md".into(),
        body: Some(body.into()),
    }
}

fn compare(before: &str, after: &str) -> Report {
    let temp = tempfile::TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let dir = temp.path().join("policy");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let store = PolicyStore::open(&dir).unwrap();
    let first = store
        .submit_desired(input(before), Submitter::Owner, NOW)
        .unwrap();
    assert!(first.report.as_ref().unwrap().ok);
    store
        .activate(
            &first.id,
            &store.effective().unwrap().effective_hash,
            "owner",
            NOW,
        )
        .unwrap();
    let second = store
        .submit_desired(input(after), Submitter::Owner, NOW)
        .unwrap();
    let report = second.report.unwrap();
    assert!(report.ok);
    // 发布时会重新校验，最终历史也必须携带同样的扩大项证据。
    let activation = store
        .activate(
            &second.id,
            &store.effective().unwrap().effective_hash,
            "owner",
            NOW,
        )
        .unwrap();
    assert_eq!(activation.expansions, report.expansions);
    report
}

#[test]
fn removed_constraint_after_display_limit_remains_visible_in_expansions() {
    let before = format!("{}禁止跳过审计。\n", "普通说明\n".repeat(2001));
    let report = compare(&before, "# 新规则\n");
    assert!(report.diff.truncated);
    assert_eq!(report.diff.entries.len(), 2000);
    assert_eq!(report.diff.removed, 2002);
    assert!(report.expansions.iter().any(|item| item.path == "old:2002"
        && item.before.as_deref() == Some("禁止跳过审计。")));
}

#[test]
fn lcs_budget_fallback_still_checks_constraints_beyond_display_limit() {
    let before = format!("{}必须保留核验。\n", "原始说明\n".repeat(2100));
    let report = compare(&before, &"全新说明\n".repeat(2101));
    assert!(report.diff.truncated);
    assert_eq!((report.diff.removed, report.diff.added), (2101, 2101));
    assert_eq!(report.expansions.len(), 1);
    assert_eq!(report.expansions[0].path, "old:2101");
}

#[test]
fn unchanged_large_rules_produce_no_replacement_or_false_expansions() {
    let body = "必须先核验。\n".repeat(3000);
    let report = compare(&body, &body);
    assert!(report.diff.entries.is_empty());
    assert!(!report.diff.truncated);
    assert!(report.expansions.is_empty());
}

#[test]
fn small_change_in_large_rules_preserves_original_line_numbers() {
    let prefix = "必须先核验。\n".repeat(2500);
    let suffix = "必须保留审计。\n".repeat(500);
    let before = format!("{prefix}禁止删除凭证。\n{suffix}");
    let after = format!("{prefix}说明调整。\n{suffix}");
    let report = compare(&before, &after);
    assert_eq!((report.diff.added, report.diff.removed), (1, 1));
    assert!(!report.diff.truncated);
    assert_eq!(report.expansions.len(), 1);
    assert_eq!(report.expansions[0].path, "old:2501");
}

#[test]
fn expansion_overflow_has_an_explicit_count_instead_of_silent_omission() {
    let report = compare(&"不得跳过核验。\n".repeat(2500), "# 新规则\n");
    assert_eq!(report.expansions.len(), 2001);
    let summary = report.expansions.last().unwrap();
    assert_eq!(summary.path, "(additional-constraints)");
    assert!(summary.note.contains("500"));
}

#[test]
fn many_empty_lines_against_one_line_do_not_allocate_a_degenerate_table() {
    let report = compare(
        &format!("# 规则\n{}禁止跳过。\n", "\n".repeat(20000)),
        "# 新规则\n",
    );
    assert!(report.diff.truncated);
    assert_eq!(report.diff.removed, 20002);
    assert_eq!(report.expansions[0].path, "old:20002");
}
