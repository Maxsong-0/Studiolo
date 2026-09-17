use super::fixture::*;
use serde_json::{Value, json};
use studiolo_host_config::Config;

#[test]
fn old_file_operation_config_never_implies_rename_or_initializes_any_storage() {
    let f = Fixture::new();
    let mut value = f.value.clone();
    value["file_operations"]
        .as_object_mut()
        .unwrap()
        .remove("rename");
    f.write(&value);
    let policy = f.policy();
    assert!(policy.rename_policy().is_none());
    assert!(policy.allows(f.vault, &path("notes/a.md")).unwrap());
    value.as_object_mut().unwrap().remove("file_operations");
    f.write(&value);
    assert!(
        Config::load(&f.path)
            .unwrap()
            .file_operation_policy()
            .unwrap()
            .is_none()
    );
    for name in ["auth", "audit"] {
        assert_eq!(std::fs::read_dir(f.root.join(name)).unwrap().count(), 0);
    }
    assert!(!f.root.join("notes").exists());
}

#[test]
fn rename_and_each_rule_reject_nonobjects_unknown_missing_and_duplicate_fields() {
    let f = Fixture::new();
    for bad in [Value::Null, json!(false), json!([]), json!([1, [], []])] {
        let mut value = f.value.clone();
        value["file_operations"]["rename"] = bad;
        f.rejects(&value);
    }
    for key in ["version", "whole_folders", "whole_files"] {
        let mut value = f.value.clone();
        value["file_operations"]["rename"]
            .as_object_mut()
            .unwrap()
            .remove(key);
        f.rejects(&value);
    }
    for bad in [
        Value::Null,
        json!([]),
        json!(["notes", "markdown_text_v1", 1]),
        json!("notes"),
    ] {
        let mut value = f.value.clone();
        value["file_operations"]["rename"]["whole_folders"][0] = bad;
        f.rejects(&value);
    }
    for key in ["path", "class", "max_bytes"] {
        let mut value = f.value.clone();
        value["file_operations"]["rename"]["whole_folders"][0]
            .as_object_mut()
            .unwrap()
            .remove(key);
        f.rejects(&value);
    }
    let mut value = f.value.clone();
    value["file_operations"]["rename"]["mime"] = json!("text/markdown");
    f.rejects(&value);
    let mut value = f.value.clone();
    value["file_operations"]["rename"]["whole_folders"][0]["mime"] = json!("text/markdown");
    f.rejects(&value);
    let encoded = serde_json::to_string(&f.value).unwrap();
    for key in ["class", "max_bytes"] {
        let field = if key == "class" {
            "\"class\":\"markdown_text_v1\""
        } else {
            "\"max_bytes\":4194304"
        };
        f.bytes(
            encoded
                .replacen(field, &format!("{field},{field}"), 1)
                .as_bytes(),
        );
        assert!(Config::load(&f.path).is_err());
    }
}

#[test]
fn class_is_an_exact_string_and_limits_are_positive_closed_values() {
    let f = Fixture::new();
    for bad in [
        json!("pdf"),
        json!("Markdown_text_v1"),
        json!({"markdown_text_v1":null}),
        json!([]),
        Value::Null,
    ] {
        let mut value = f.value.clone();
        value["file_operations"]["rename"]["whole_folders"][0]["class"] = bad;
        f.rejects(&value);
    }
    for bad in [
        json!(0),
        json!(4194305),
        json!(u64::MAX),
        json!(-1),
        json!(1.5),
        json!("1"),
    ] {
        let mut value = f.value.clone();
        value["file_operations"]["rename"]["whole_folders"][0]["max_bytes"] = bad;
        f.rejects(&value);
    }
    let mut value = f.value.clone();
    value["file_operations"]["rename"]["version"] = json!(2);
    f.rejects(&value);
    let mut value = f.value.clone();
    value["file_operations"]["rename"]["whole_folders"] = json!([]);
    f.rejects(&value);
}
