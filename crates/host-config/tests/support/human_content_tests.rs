//! 可信范围从私有配置加载；文件名、前缀和自声明不能绕过固定保护。
use super::*;
use studiolo_core::{VaultPath, VaultUuid};

fn document(vault: VaultUuid) -> Value {
    json!({"version":1,"vault_uuid":vault,"folders":["notes"],"protected":["notes/private"],
        "documents":{"project/INDEX.md":[["<!-- generated:start -->","<!-- generated:end -->"]]}})
}

fn scope(f: &Fixture) -> studiolo_host_config::HumanContentScope {
    for name in ["vault", "assets", "data"] {
        private_dir(&f.root.join(name));
    }
    Config::load(&f.path)
        .unwrap()
        .canonical_serving_roots(
            &f.root.join("vault"),
            &f.root.join("assets"),
            &f.root.join("data"),
        )
        .unwrap()
        .human_content()
        .unwrap()
        .clone()
}
fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

#[test]
fn scope_requires_matching_vault_and_component_bounded_prefix() {
    let f = Fixture::new();
    let vault = VaultUuid::new();
    let mut value = f.value.clone();
    value["human_content"] = document(vault);
    f.write(&value);
    let scope = scope(&f);
    assert!(scope.allows(vault, &path("notes/plain.md")));
    for name in [
        "notes-other/plain.md",
        "notes/private/a.md",
        "notes/CLAUDE.md",
        "notes/skills/a.md",
        "notes/ops/a.md",
        "notes/INDEX.md",
        "notes/TASKS.md",
        "notes/sources/a.md",
    ] {
        assert!(!scope.allows(vault, &path(name)), "{name}");
    }
    assert!(!scope.allows(VaultUuid::new(), &path("notes/plain.md")));
    assert!(scope.allows(vault, &path("project/INDEX.md")));
}

#[test]
fn generated_regions_preserve_exact_bytes_but_allow_surrounding_prose() {
    let f = Fixture::new();
    let mut value = f.value.clone();
    value["human_content"] = document(VaultUuid::new());
    f.write(&value);
    let scope = scope(&f);
    let path = path("project/INDEX.md");
    let before = "human\n<!-- generated:start -->\ncomputed\n<!-- generated:end -->\ntail\n";
    assert!(scope.preserves_generated(&path, before, &before.replace("human", "changed")));
    for after in [
        before.replace("computed", "different"),
        before.replace("<!-- generated:start -->", ""),
        format!("{before}{before}"),
        before.replace("<!-- generated:end -->", "<!-- generated:end -->\r"),
    ] {
        assert!(!scope.preserves_generated(&path, before, &after));
    }
}

#[test]
fn unknown_fields_array_shapes_and_unbounded_or_protected_grants_are_rejected() {
    let f = Fixture::new();
    for candidate in [
        Value::Null,
        json!([]),
        json!({"version":1,"folders":[]}),
        {
            let mut v = document(VaultUuid::new());
            v["force"] = true.into();
            v
        },
        {
            let mut v = document(VaultUuid::new());
            v["folders"] = json!(["_规则"]);
            v
        },
        {
            let mut v = document(VaultUuid::new());
            v["documents"] = json!({"CLAUDE.md":[]});
            v
        },
        {
            let mut v = document(VaultUuid::new());
            v["documents"] = json!({"a.md":[["x","y"]]});
            v
        },
        {
            let mut v = document(VaultUuid::new());
            v["documents"] = json!({"a.md":[["<!-- same -->","<!-- same -->"]]});
            v
        },
    ] {
        let mut value = f.value.clone();
        value["human_content"] = candidate;
        f.rejects(&value);
    }
}

#[test]
fn changed_scope_cannot_reauthorize_an_already_bound_running_instance() {
    let f = Fixture::new();
    let vault = VaultUuid::new();
    let mut value = f.value.clone();
    value["human_content"] = document(vault);
    f.write(&value);
    for name in ["vault", "assets", "data"] {
        private_dir(&f.root.join(name));
    }
    let roots = Config::load(&f.path)
        .unwrap()
        .canonical_serving_roots(
            &f.root.join("vault"),
            &f.root.join("assets"),
            &f.root.join("data"),
        )
        .unwrap();
    value["human_content"]["folders"] = json!(["other"]);
    f.write(&value);
    assert!(roots.revalidate().is_err());
    assert!(
        !roots
            .human_content()
            .unwrap()
            .allows(vault, &path("other/a.md"))
    );
}
