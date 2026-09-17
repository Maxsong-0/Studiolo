use super::fixture::*;
use serde_json::json;
use studiolo_host_config::Config;

#[test]
fn folder_file_overlaps_and_case_or_unicode_prefix_aliases_are_rejected() {
    let f = Fixture::new();
    for folders in [
        json!([folder("笔记"), folder("笔记/sub")]),
        json!([folder("notes/a"), folder("Notes/b")]),
        json!([folder("Straße/a"), folder("STRASSE/b")]),
        json!([folder("笔记"), folder("笔记")]),
    ] {
        let mut value = f.value.clone();
        value["sync_scope"]["whole_folders"] = folders;
        f.write_value(&value);
        assert!(f.load().is_err());
    }
    for exact in ["笔记", "笔记/sub/file.md", "附件"] {
        let mut value = f.value.clone();
        value["sync_scope"]["whole_files"]
            .as_array_mut()
            .unwrap()
            .push(file(exact, true, &[]));
        f.write_value(&value);
        assert!(f.load().is_err());
    }
}

#[test]
fn noncanonical_and_invisible_paths_fail_before_any_content_read() {
    let f = Fixture::new();
    for bad in [
        "/绝对",
        "../父级",
        "a//b",
        "e\u{301}",
        "笔记/\u{202e}隐藏",
        "笔记/隐藏\u{fe0f}",
        "笔记/隐藏\u{e0100}",
        "笔记/a.",
        "笔记/a ",
        "a:b",
    ] {
        let mut value = f.value.clone();
        value["sync_scope"]["whole_folders"][0]["prefix"] = json!(bad);
        f.write_value(&value);
        assert!(f.load().is_err(), "{bad}");
    }
}

#[test]
fn sync_addition_does_not_expand_existing_human_edit_scope() {
    let mut f = Fixture::new();
    f.value["human_content"] = json!({"version":1,"vault_uuid":f.vault,
        "folders":["浏览器"],"protected":[],"documents":{}});
    f.write();
    for name in ["vault", "assets", "writer"] {
        directory(&f.root.join(name));
    }
    let config = Config::load(&f.path).unwrap();
    let roots = config
        .canonical_serving_roots(
            &f.root.join("vault"),
            &f.root.join("assets"),
            &f.root.join("writer"),
        )
        .unwrap();
    let human = roots.human_content().unwrap();
    assert!(human.allows(f.vault, &path("浏览器/a.md")));
    assert!(!human.allows(f.vault, &path("笔记/a.md")));
    let scope = config.sync_scope().unwrap().unwrap();
    let item = markdown("笔记/a.md");
    assert!(
        scope
            .approve_union(
                scope.binding(),
                &[],
                std::slice::from_ref(&item),
                std::slice::from_ref(&item)
            )
            .is_ok()
    );
    let browser = markdown("浏览器/a.md");
    assert!(
        scope
            .approve_union(
                scope.binding(),
                &[],
                std::slice::from_ref(&browser),
                std::slice::from_ref(&browser)
            )
            .is_err()
    );
}
