//! 先查可信路径约束再读取正文，不伪造全量 Missing 观察。

use super::fixture::*;
use serde_json::json;
use std::{fs, os::unix::fs::symlink};
use studiolo_host_config::{SyncAttachmentType as Attachment, SyncScopeError as Error};

#[test]
fn returns_registered_policy_without_needing_or_reading_content() {
    let f = Fixture::new();
    let scope = f.scope();
    let markdown = scope
        .policy_for_observation(scope.binding(), &path("笔记/尚未创建.md"))
        .unwrap()
        .unwrap();
    assert!(markdown.markdown());
    assert!(markdown.attachment_types().is_empty());
    assert_eq!(markdown.max_bytes(), 1048576);
    let pdf = scope
        .policy_for_observation(scope.binding(), &path("附件/手册"))
        .unwrap()
        .unwrap();
    assert!(!pdf.markdown());
    assert_eq!(pdf.attachment_types(), &[Attachment::Pdf]);
    assert!(!f.root.join("笔记").exists());
    assert!(!f.root.join("附件").exists());
    for name in ["auth", "audit"] {
        assert_eq!(fs::read_dir(f.root.join(name)).unwrap().count(), 0);
    }
}

#[test]
fn excluded_and_unapproved_paths_return_none_instead_of_a_read_policy() {
    let f = Fixture::new();
    let scope = f.scope();
    for name in [
        "不批准/a.md",
        "笔记外/a.md",
        "笔记",
        "笔记/TASKS.md",
        "笔记/INDEX.md",
        "笔记/dashboard.md",
        "笔记/自动汇总.md",
        "笔记/自动汇总.md/child.md",
        "笔记/.private/key",
        "笔记/ops/tasks/a.md",
        "笔记/raw/a.md",
        "附件/未登记.pdf",
    ] {
        assert!(
            scope
                .policy_for_observation(scope.binding(), &path(name))
                .unwrap()
                .is_none(),
            "{name}"
        );
    }
}

#[test]
fn aliases_and_malformed_paths_are_errors_even_when_the_path_would_be_excluded() {
    let mut f = Fixture::new();
    f.value["sync_scope"]["whole_folders"] = json!([folder("notes")]);
    f.value["sync_scope"]["generated_documents"] = json!(["notes/Generated.md"]);
    f.write();
    let scope = f.scope();
    for name in [
        "Notes/plain.md",
        "Notes/TASKS.md",
        "notes/generated.md",
        "notes/Generated.md/\u{202e}child.md",
        "notes/TASKS\u{fe0f}.md",
        "outside/trailing.",
    ] {
        assert!(
            matches!(
                scope.policy_for_observation(scope.binding(), &path(name)),
                Err(Error::Collision)
            ),
            "{name}"
        );
    }
    // 同一规范父目录下大小写不同的固定保留 basename 仍被硬排除。
    assert!(
        scope
            .policy_for_observation(scope.binding(), &path("notes/tasks.MD"))
            .unwrap()
            .is_none()
    );
    assert!(studiolo_core::VaultPath::parse("notes/e\u{301}.md").is_err());
}

#[test]
fn unicode_aliases_and_single_path_budgets_match_complete_union_checks() {
    let mut f = Fixture::new();
    f.value["sync_scope"]["whole_folders"] = json!([folder("Straße")]);
    f.value["sync_scope"]["generated_documents"] = json!([]);
    f.write();
    let scope = f.scope();
    assert!(matches!(
        scope.policy_for_observation(scope.binding(), &path("STRASSE/a.md")),
        Err(Error::Collision)
    ));
    for name in [
        format!("Straße/{}", "a".repeat(1024)),
        format!("Straße/{}", "a/".repeat(64) + "file.md"),
    ] {
        assert!(matches!(
            scope.policy_for_observation(scope.binding(), &path(&name)),
            Err(Error::Budget)
        ));
    }
}

#[test]
fn one_path_policy_is_not_missing_evidence_or_permission_to_omit_old_baseline() {
    let f = Fixture::new();
    let scope = f.scope();
    assert!(
        scope
            .policy_for_observation(scope.binding(), &path("笔记/a.md"))
            .unwrap()
            .is_some()
    );
    let old = vec![markdown("笔记/自动汇总.md")];
    assert!(
        scope
            .policy_for_observation(scope.binding(), &old[0].path)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        scope.approve_union(scope.binding(), &old, &[], &[]),
        Err(Error::ScopeChangeRequired)
    ));
    let approved_old = vec![markdown("笔记/a.md")];
    assert!(matches!(
        scope.approve_union(scope.binding(), &approved_old, &[], &[]),
        Err(Error::Incomplete)
    ));
}

#[test]
fn binding_and_private_snapshot_changes_invalidate_observation_policies() {
    let f = Fixture::new();
    let scope = f.scope();
    let item = path("笔记/a.md");
    let mut binding = scope.binding().clone();
    binding.scope_hash[0] ^= 1;
    assert!(matches!(
        scope.policy_for_observation(&binding, &item),
        Err(Error::BindingMismatch)
    ));
    let other = f.root.join("config/original.json");
    fs::rename(&f.path, &other).unwrap();
    symlink(&other, &f.path).unwrap();
    assert!(matches!(
        scope.policy_for_observation(scope.binding(), &item),
        Err(Error::ConfigurationChanged)
    ));
}
