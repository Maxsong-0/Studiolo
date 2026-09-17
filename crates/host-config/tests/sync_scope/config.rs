use super::fixture::*;
use serde_json::{Value, json};

#[test]
fn absent_scope_is_disabled_without_reading_any_scope_or_serving_root() {
    let mut f = Fixture::new();
    f.value.as_object_mut().unwrap().remove("sync_scope");
    f.write();
    let config = f.load().unwrap();
    std::fs::remove_file(&f.path).unwrap();
    assert!(config.sync_scope().unwrap().is_none());
    for name in ["auth", "audit"] {
        assert_eq!(std::fs::read_dir(f.root.join(name)).unwrap().count(), 0);
    }
}

#[test]
fn rules_require_objects_explicit_types_budgets_and_known_fields() {
    let f = Fixture::new();
    for bad in [Value::Null, json!([]), json!("scope"), json!(false)] {
        let mut value = f.value.clone();
        value["sync_scope"] = bad;
        f.write_value(&value);
        assert!(f.load().is_err());
    }
    for pointer in ["whole_folders", "whole_files"] {
        for bad in [
            json!([null]),
            json!([["笔记", true, [], 1024]]),
            json!([{}]),
        ] {
            let mut value = f.value.clone();
            value["sync_scope"][pointer] = bad;
            f.write_value(&value);
            assert!(f.load().is_err());
        }
        for field in ["markdown", "attachment_types", "max_bytes"] {
            let mut value = f.value.clone();
            value["sync_scope"][pointer][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            f.write_value(&value);
            assert!(f.load().is_err());
        }
    }
    for (field, bad) in [
        ("max_bytes", json!(0)),
        ("max_bytes", json!(268435457)),
        ("attachment_types", json!(["pdf", "pdf"])),
        ("attachment_types", json!(["auto"])),
        ("markdown", json!("true")),
        ("whole_page", json!(true)),
    ] {
        let mut value = f.value.clone();
        value["sync_scope"]["whole_folders"][0][field] = bad;
        f.write_value(&value);
        assert!(f.load().is_err());
    }
}

#[test]
fn all_disabled_types_and_duplicate_json_fields_fail_closed() {
    let f = Fixture::new();
    let mut value = f.value.clone();
    value["sync_scope"]["whole_folders"][0]["markdown"] = json!(false);
    f.write_value(&value);
    assert!(f.load().is_err());
    let encoded = serde_json::to_string(&f.value).unwrap();
    for duplicate in [
        encoded.replacen(
            "\"scope_epoch\":1",
            "\"scope_epoch\":1,\"scope_epoch\":1",
            1,
        ),
        encoded.replacen(
            "\"markdown\":true",
            "\"markdown\":true,\"markdown\":true",
            1,
        ),
    ] {
        f.bytes(duplicate.as_bytes());
        assert!(f.load().is_err());
    }
}

#[test]
fn identities_epochs_and_registered_generation_paths_are_bounded() {
    let f = Fixture::new();
    for (key, invalid) in [
        ("version", json!(2)),
        ("vault_uuid", json!("bad")),
        ("link_id", json!("00000000000000000000000000000000")),
        ("link_id", json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")),
        ("remote_vault", json!("2")),
        ("scope_epoch", json!(0)),
        ("scope_epoch", json!(u64::MAX)),
        ("generated_documents", json!(["笔记/a", "笔记/a"])),
    ] {
        let mut value = f.value.clone();
        value["sync_scope"][key] = invalid;
        f.write_value(&value);
        assert!(f.load().is_err());
    }
}
