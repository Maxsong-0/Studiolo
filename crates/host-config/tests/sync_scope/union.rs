use super::fixture::*;
use studiolo_host_config::{
    SyncAttachmentType as Attachment, SyncFileKind as Kind, SyncScopeError as Error,
};

#[test]
fn complete_union_is_sorted_and_preserves_exact_explicit_per_file_rules() {
    let f = Fixture::new();
    let scope = f.scope();
    let base = vec![markdown("笔记/已删除.md")];
    let canonical = vec![
        markdown("笔记/新建.md"),
        missing("笔记/已删除.md"),
        present("附件/手册", Kind::Attachment(Attachment::Pdf), 1048576),
    ];
    let mirror = vec![
        missing("笔记/新建.md"),
        missing("笔记/已删除.md"),
        present("附件/手册", Kind::Attachment(Attachment::Pdf), 1048576),
    ];
    let result = scope
        .approve_union(scope.binding(), &base, &canonical, &mirror)
        .unwrap();
    assert_eq!(result.binding(), scope.binding());
    assert_eq!(
        result
            .paths()
            .keys()
            .map(|p| p.as_str())
            .collect::<Vec<_>>(),
        vec!["笔记/已删除.md", "笔记/新建.md", "附件/手册"]
    );
    assert!(result.paths()[&path("笔记/新建.md")].markdown());
    let attachment = &result.paths()[&path("附件/手册")];
    assert!(!attachment.markdown());
    assert_eq!(attachment.attachment_types(), &[Attachment::Pdf]);
    assert_eq!(attachment.max_bytes(), 1048576);
}

#[test]
fn baseline_paths_are_not_omitted_or_treated_as_approved_deletions() {
    let f = Fixture::new();
    let scope = f.scope();
    let old = markdown("笔记/以前.md");
    assert!(matches!(
        scope.approve_union(scope.binding(), &[old], &[], &[]),
        Err(Error::Incomplete)
    ));
    for excluded in ["不再批准/以前.md", "笔记/TASKS.md", "笔记/自动汇总.md"] {
        let old = markdown(excluded);
        let current = missing(excluded);
        assert!(matches!(
            scope.approve_union(
                scope.binding(),
                &[old],
                std::slice::from_ref(&current),
                std::slice::from_ref(&current)
            ),
            Err(Error::ScopeChangeRequired)
        ));
    }
}

#[test]
fn unapproved_missing_entries_fail_in_each_of_the_three_inputs() {
    let f = Fixture::new();
    let scope = f.scope();
    let good = vec![markdown("笔记/a.md")];
    let bad = vec![missing("笔记外/b.md")];
    for (baseline, canonical, mirror) in [
        (&bad, &good, &good),
        (&good, &bad, &good),
        (&good, &good, &bad),
    ] {
        assert!(matches!(
            scope.approve_union(scope.binding(), baseline, canonical, mirror),
            Err(Error::ScopeChangeRequired)
        ));
    }
}

#[test]
fn one_current_side_cannot_omit_an_explicit_missing_union_entry() {
    let f = Fixture::new();
    let scope = f.scope();
    let current = [markdown("笔记/a.md")];
    for (canonical, mirror) in [(&current[..], &[][..]), (&[][..], &current[..])] {
        assert!(matches!(
            scope.approve_union(scope.binding(), &[], canonical, mirror),
            Err(Error::Incomplete)
        ));
    }
}

#[test]
fn empty_approved_scope_observation_is_distinct_from_missing_baseline_evidence() {
    let f = Fixture::new();
    let scope = f.scope();
    assert!(
        scope
            .approve_union(scope.binding(), &[], &[], &[])
            .unwrap()
            .paths()
            .is_empty()
    );
    let missing = vec![missing("笔记/a.md")];
    assert_eq!(
        scope
            .approve_union(scope.binding(), &missing, &missing, &missing)
            .unwrap()
            .paths()
            .len(),
        1
    );
}

#[test]
fn type_is_explicit_not_inferred_from_filename_or_attachment_suffix() {
    let f = Fixture::new();
    let scope = f.scope();
    for bad in [
        present("附件/手册", Kind::Markdown, 2),
        present("附件/手册", Kind::Attachment(Attachment::Jpeg), 2),
        present("笔记/paper.pdf", Kind::Attachment(Attachment::Pdf), 2),
    ] {
        assert!(matches!(
            scope.approve_union(
                scope.binding(),
                &[],
                std::slice::from_ref(&bad),
                std::slice::from_ref(&bad)
            ),
            Err(Error::ScopeChangeRequired)
        ));
    }
    let ordinary = markdown("笔记/没有扩展名");
    assert!(
        scope
            .approve_union(
                scope.binding(),
                &[],
                std::slice::from_ref(&ordinary),
                std::slice::from_ref(&ordinary)
            )
            .is_ok()
    );
}

#[test]
fn binding_requires_vault_link_remote_epoch_and_actual_config_hash() {
    let f = Fixture::new();
    let scope = f.scope();
    for index in 0..5 {
        let mut binding = scope.binding().clone();
        match index {
            0 => binding.vault_uuid = studiolo_core::VaultUuid::new(),
            1 => binding.link_id[0] ^= 1,
            2 => binding.remote_vault[0] ^= 1,
            3 => binding.scope_epoch += 1,
            _ => binding.scope_hash[0] ^= 1,
        }
        assert!(matches!(
            scope.approve_union(&binding, &[], &[], &[]),
            Err(Error::BindingMismatch)
        ));
    }
}

#[test]
fn duplicate_case_unicode_and_file_directory_collisions_fail_as_one_batch() {
    let f = Fixture::new();
    let scope = f.scope();
    for names in [
        vec!["笔记/a.md", "笔记/a.md"],
        vec!["笔记/Dir/a.md", "笔记/dir/b.md"],
        vec!["笔记/Straße/a.md", "笔记/STRASSE/b.md"],
        vec!["笔记/σ/a.md", "笔记/ς/b.md"],
        vec!["笔记/a.md", "笔记/a.md/b.md"],
    ] {
        let items: Vec<_> = names.into_iter().map(markdown).collect();
        assert!(matches!(
            scope.approve_union(scope.binding(), &[], &items, &items),
            Err(Error::Collision)
        ));
    }
}

#[test]
fn collisions_are_detected_across_snapshots_and_against_approved_prefix_spelling() {
    let mut f = Fixture::new();
    f.value["sync_scope"]["whole_folders"] = serde_json::json!([folder("notes")]);
    f.value["sync_scope"]["generated_documents"] = serde_json::json!([]);
    f.write();
    let scope = f.scope();
    let old = vec![markdown("notes/Dir/a.md")];
    let current = vec![markdown("notes/dir/b.md")];
    assert!(matches!(
        scope.approve_union(scope.binding(), &old, &current, &current),
        Err(Error::Collision)
    ));
    let wrong_spelling = vec![missing("Notes/a.md")];
    assert!(matches!(
        scope.approve_union(scope.binding(), &[], &wrong_spelling, &wrong_spelling),
        Err(Error::Collision)
    ));
}
