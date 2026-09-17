use super::fixture::*;
use serde_json::json;
use studiolo_host_config::Config;

#[test]
fn each_rule_count_has_its_own_bound_and_actual_whole_config_is_still_sixteen_kib() {
    let f = Fixture::new();
    for (field, count) in [("whole_folders", 64), ("whole_files", 128)] {
        let mut value = f.value.clone();
        value["file_operations"]["rename"] =
            json!({"version":1,"whole_folders":[],"whole_files":[]});
        value["file_operations"]["rename"][field] = json!(
            (0..count)
                .map(|n| rule(&format!("notes/f{n}.md"), 100))
                .collect::<Vec<_>>()
        );
        f.write(&value);
        assert!(Config::load(&f.path).is_ok(), "{field}");
        value["file_operations"]["rename"][field]
            .as_array_mut()
            .unwrap()
            .push(rule("notes/extra.md", 100));
        f.rejects(&value);
    }
    let bytes = serde_json::to_vec(&f.value).unwrap();
    let mut exact = bytes.clone();
    exact.resize(16_384, b' ');
    f.bytes(&exact);
    assert!(Config::load(&f.path).is_ok());
    exact.push(b' ');
    f.bytes(&exact);
    assert!(Config::load(&f.path).is_err());
}

#[test]
fn rename_accepts_4096_path_bytes_but_sync_keeps_its_independent_1024_limit() {
    let f = Fixture::new();
    let parent = f.policy();
    let rename = parent.rename_policy().unwrap();
    let name = format!("notes/{}.md", "a".repeat(4087));
    assert_eq!(name.len(), 4096);
    assert!(
        rename
            .approve_pair(f.vault, &path(&name), &path("other/b.md"))
            .is_ok()
    );
    let over = format!("notes/{}.md", "a".repeat(4088));
    assert!(
        rename
            .approve_pair(f.vault, &path(&over), &path("other/b.md"))
            .is_err()
    );
    let mut value = f.value.clone();
    value["sync_scope"] = sync(&f, &[]);
    let name = format!("sync/{}.md", "a".repeat(1016));
    assert_eq!(name.len(), 1024);
    value["sync_scope"]["whole_files"][0]["path"] = json!(name);
    f.write(&value);
    assert!(Config::load(&f.path).is_ok());
    value["sync_scope"]["whole_files"][0]["path"] = json!(format!("sync/{}.md", "a".repeat(1017)));
    f.rejects(&value);
}

#[test]
fn path_depth_budget_applies_to_both_members_before_pair_publication() {
    let f = Fixture::new();
    let parent = f.policy();
    let rename = parent.rename_policy().unwrap();
    let within = format!("notes/{}a.md", "x/".repeat(62));
    assert_eq!(within.split('/').count(), 64);
    rename
        .approve_pair(f.vault, &path(&within), &path("other/a.md"))
        .unwrap();
    let over = format!("notes/{}a.md", "x/".repeat(63));
    assert!(
        rename
            .approve_pair(f.vault, &path(&over), &path("other/a.md"))
            .is_err()
    );
    assert!(
        rename
            .approve_pair(f.vault, &path("other/a.md"), &path(&over))
            .is_err()
    );
}
