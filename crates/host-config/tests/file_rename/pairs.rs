use super::fixture::*;
use serde_json::json;
use studiolo_core::VaultUuid;

#[test]
fn both_paths_are_frozen_to_the_parent_binding_and_minimum_limit_without_file_io() {
    let f = Fixture::new();
    let parent = f.policy();
    let pair = parent
        .rename_policy()
        .unwrap()
        .approve_pair(
            f.vault,
            &path("notes/中文v1.2.md"),
            &path("other/archive.md"),
        )
        .unwrap();
    assert_eq!(pair.vault_uuid(), f.vault);
    assert_eq!(pair.from_path(), &path("notes/中文v1.2.md"));
    assert_eq!(pair.to_path(), &path("other/archive.md"));
    assert_eq!(pair.class(), "markdown_text_v1");
    assert_eq!(pair.mime(), "text/markdown; charset=utf-8");
    assert_eq!(pair.extension(), ".md");
    assert_eq!(pair.max_bytes(), 2048);
    assert_eq!(pair.scope_hash(), parent.scope_hash());
    assert_eq!(pair.scope_epoch(), parent.scope_epoch());
    pair.revalidate().unwrap();
    assert!(!f.root.join("notes").exists());
    let mut value = f.value.clone();
    value["file_operations"]["effective_object_limit"] = json!(511);
    f.write(&value);
    let parent = f.policy();
    assert_eq!(
        parent
            .rename_policy()
            .unwrap()
            .approve_pair(f.vault, &path("notes/a.md"), &path("other/b.md"))
            .unwrap()
            .max_bytes(),
        511
    );
}

#[test]
fn exact_files_cannot_grant_siblings_and_same_suffix_never_grants_an_unregistered_path() {
    let f = Fixture::new();
    let mut value = f.value.clone();
    value["file_operations"]["rename"] = json!({"version":1,"whole_folders":[],"whole_files":[rule("one.md",100),rule("two.md",80)]});
    f.write(&value);
    let parent = f.policy();
    let policy = parent.rename_policy().unwrap();
    assert_eq!(
        policy
            .approve_pair(f.vault, &path("one.md"), &path("two.md"))
            .unwrap()
            .max_bytes(),
        80
    );
    for (from, to) in [
        ("notes/a.md", "two.md"),
        ("one.md", "notes/a.md"),
        ("one.md", "one.md"),
        ("one.md", "one.md/child.md"),
    ] {
        assert!(
            policy
                .approve_pair(f.vault, &path(from), &path(to))
                .is_err()
        );
    }
    assert!(
        policy
            .approve_pair(VaultUuid::new(), &path("one.md"), &path("two.md"))
            .is_err()
    );
}

#[test]
fn exact_suffix_and_shared_prefix_index_reject_aliases_but_allow_version_dots() {
    let f = Fixture::new();
    let parent = f.policy();
    let policy = parent.rename_policy().unwrap();
    policy
        .approve_pair(f.vault, &path("notes/a.pdf.md"), &path("notes/v1.2.md"))
        .unwrap();
    for to in [
        "notes/a.MD",
        "notes/a.md.txt",
        "notes/a.md ",
        "notes/a.md.",
        "notes/.md",
        "notes/a\u{feff}.md",
        "notes/a\u{202e}.md",
        "Notes/new.md",
        "notes/A.md",
        "notes/a.md/b.md",
    ] {
        assert!(
            policy
                .approve_pair(f.vault, &path("notes/a.md"), &path(to))
                .is_err(),
            "{to}"
        );
    }
    assert!(
        policy
            .approve_pair(
                f.vault,
                &path("notes/Straße/a.md"),
                &path("notes/STRASSE/b.md")
            )
            .is_err()
    );
}
