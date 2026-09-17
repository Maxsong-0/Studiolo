//! 复用单个测试私有配置夹具；没有生产 scanner、正文读取或外部服务。
#[path = "sync_scope/fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "sync_traversal/types.rs"]
mod types;
use fixture::*;
use serde_json::json;
use studiolo_host_config::{SyncDirectoryDecision as Decision, SyncScopeError as Error};

#[test]
fn selection_has_read_only_anchors_files_exclusions_and_original_binding() {
    let f = Fixture::new();
    let scope = f.scope();
    let selection = scope.observation_selection(scope.binding()).unwrap();
    assert_eq!(selection.binding(), scope.binding());
    assert_eq!(selection.folders()[0].path(), &path("笔记"));
    assert_eq!(selection.files()[0].path(), &path("附件/手册"));
    assert_eq!(selection.folders()[0].policy().max_bytes(), 1048576);
    assert!(
        selection
            .excluded_prefixes()
            .contains(&path("笔记/自动汇总.md"))
    );
}

#[test]
fn folder_anchor_and_exact_file_ancestors_traverse_but_file_none_is_not_directory_skip() {
    let f = Fixture::new();
    let scope = f.scope();
    assert!(
        scope
            .policy_for_observation(scope.binding(), &path("笔记"))
            .unwrap()
            .is_none()
    );
    for name in ["笔记", "笔记/深层", "附件"] {
        assert_eq!(
            scope
                .directory_for_observation(scope.binding(), &path(name))
                .unwrap(),
            Decision::Traverse
        );
    }
    for name in [
        "附件/手册",
        "其它",
        "笔记/TASKS.md",
        "笔记/自动汇总.md",
        "笔记/raw",
    ] {
        assert_eq!(
            scope
                .directory_for_observation(scope.binding(), &path(name))
                .unwrap(),
            Decision::Skip
        );
    }
}

#[test]
fn directory_aliases_binding_and_configuration_changes_fail_before_selection() {
    let mut f = Fixture::new();
    f.value["sync_scope"]["whole_folders"] = json!([folder("Notes")]);
    f.write();
    let scope = f.scope();
    assert!(matches!(
        scope.directory_for_observation(scope.binding(), &path("notes")),
        Err(Error::Collision)
    ));
    let mut binding = scope.binding().clone();
    binding.scope_epoch += 1;
    assert!(matches!(
        scope.observation_selection(&binding),
        Err(Error::BindingMismatch)
    ));
    f.value["sync_scope"]["scope_epoch"] = json!(2);
    f.write();
    assert!(matches!(
        scope.observation_selection(scope.binding()),
        Err(Error::ConfigurationChanged)
    ));
    assert!(matches!(
        scope.directory_for_observation(scope.binding(), &path("Notes")),
        Err(Error::ConfigurationChanged)
    ));
}
