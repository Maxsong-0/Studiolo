use super::*;
use serde_json::Value;

#[test]
fn all_nine_attachment_strings_are_accepted_without_external_tagged_enum_objects() {
    let f = Fixture::new();
    for kind in [
        "png", "jpeg", "gif", "webp", "pdf", "mp3", "mp4", "wav", "zip",
    ] {
        let mut value = f.value.clone();
        value["sync_scope"]["whole_files"][0]["attachment_types"] = json!([kind]);
        f.write_value(&value);
        assert!(f.load().is_ok(), "{kind}");
    }
}

#[test]
fn object_array_null_and_nonstring_attachment_items_are_rejected() {
    let f = Fixture::new();
    for bad in [
        json!({"pdf":null}),
        json!({"png":[]}),
        json!(["pdf"]),
        Value::Null,
        json!(1),
        json!(true),
    ] {
        let mut value = f.value.clone();
        value["sync_scope"]["whole_files"][0]["attachment_types"] = json!([bad]);
        f.write_value(&value);
        assert!(f.load().is_err());
    }
}
