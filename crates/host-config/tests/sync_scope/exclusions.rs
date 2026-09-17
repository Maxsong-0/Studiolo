use super::fixture::*;
use serde_json::json;
use std::fs;
use studiolo_host_config::SyncScopeError as Error;

#[test]
fn hard_exclusions_apply_equally_to_canonical_mirror_and_missing_paths() {
    let f = Fixture::new();
    let scope = f.scope();
    for suffix in [
        "TASKS.md",
        "INDEX.md",
        "dashboard.md",
        "tasks.MD",
        "AGENTS.md",
        "CLAUDE.md",
        "SKILL.md",
        ".private/key",
        ".workbench/state",
        ".obsidian/workspace",
        "ops/tasks/a.md",
        "ops/approvals/a.md",
        "ops/log/a.md",
        "WORKBENCH_DATA/state",
        "workbench_ephemeral/cache",
        "sources/x/raw/a",
        "raw/a.md",
        "skills/a.md",
        "policies/a.md",
        "capabilities/a.md",
        "_规则/a.md",
    ] {
        let name = format!("笔记/{suffix}");
        let item = markdown(&name);
        let absent = missing(&name);
        for (left, right) in [(&item, &absent), (&absent, &item), (&absent, &absent)] {
            assert!(
                matches!(
                    scope.approve_union(
                        scope.binding(),
                        &[],
                        std::slice::from_ref(left),
                        std::slice::from_ref(right)
                    ),
                    Err(Error::ScopeChangeRequired)
                ),
                "{name}"
            );
        }
    }
}

#[test]
fn exact_file_approval_cannot_override_reserved_or_registered_generated_paths() {
    let f = Fixture::new();
    for name in [
        "TASKS.md",
        "INDEX.md",
        "dashboard.md",
        "skills/permit.md",
        "笔记/自动汇总.md",
    ] {
        let mut value = f.value.clone();
        value["sync_scope"]["whole_folders"] = json!([]);
        value["sync_scope"]["whole_files"] = json!([file(name, true, &[])]);
        f.write_value(&value);
        assert!(f.load().is_err(), "{name}");
    }
}

#[test]
fn generated_registration_survives_removed_sentinels_and_retains_human_tasks_bytes() {
    let mut f = Fixture::new();
    f.value["human_content"] = json!({"version":1,"vault_uuid":f.vault,"folders":["笔记"],
        "protected":["笔记/私有"],"documents":{"笔记/混合.md":[["<!-- begin -->","<!-- end -->"]],
            "笔记/TASKS.md":[]}});
    f.write();
    let scope = f.scope();
    let vault = f.root.join("content");
    directory(&vault);
    let tasks = vault.join("TASKS.md");
    let human = "# 工作安排\n| 明天写方案 | 由我认领 | 进行中 |\n";
    fs::write(&tasks, human).unwrap();
    let mixed = vault.join("混合.md");
    // 真实正文已无哨兵；范围判定只读可信登记，不能因正文自删哨兵而放行。
    fs::write(&mixed, "全部哨兵已经删除\n").unwrap();
    for name in [
        "笔记/混合.md",
        "笔记/混合.md/child.md",
        "笔记/TASKS.md",
        "笔记/私有/a.md",
    ] {
        let item = markdown(name);
        assert!(matches!(
            scope.approve_union(
                scope.binding(),
                &[],
                std::slice::from_ref(&item),
                std::slice::from_ref(&item)
            ),
            Err(Error::ScopeChangeRequired)
        ));
    }
    assert_eq!(fs::read_to_string(&tasks).unwrap(), human);
    assert_eq!(fs::read_to_string(&mixed).unwrap(), "全部哨兵已经删除\n");
}

#[test]
fn human_generated_registry_cannot_silently_bind_another_vault_or_alias() {
    let f = Fixture::new();
    for (vault, generated) in [
        (studiolo_core::VaultUuid::new(), "笔记/混合.md"),
        (f.vault, "笔记/dir/混合.md"),
    ] {
        let mut value = f.value.clone();
        value["human_content"] = json!({"version":1,"vault_uuid":vault,"folders":["笔记"],
            "protected":[],"documents":{generated:[["<!-- begin -->","<!-- end -->"]]}});
        value["sync_scope"]["whole_files"] = json!([file("笔记/Dir/其他.md", true, &[])]);
        value["sync_scope"]["whole_folders"] = json!([]);
        f.write_value(&value);
        assert!(f.load().is_err());
    }
}

#[test]
fn folder_prefix_is_component_bounded_and_cannot_be_used_as_a_file() {
    let f = Fixture::new();
    let scope = f.scope();
    for name in ["笔记外/a.md", "笔记"] {
        let item = missing(name);
        assert!(matches!(
            scope.approve_union(
                scope.binding(),
                &[],
                std::slice::from_ref(&item),
                std::slice::from_ref(&item)
            ),
            Err(Error::ScopeChangeRequired)
        ));
    }
}
