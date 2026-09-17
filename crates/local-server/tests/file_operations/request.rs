use super::*;
use serde_json::{Value, json};

fn body() -> Value {
    json!({"kind":"delete","vault_uuid":"57c07e80-49f2-4493-9268-a409d473701a",
    "path":"files/附件.bin","expected_revision":"a".repeat(64),"idempotency_key":"file-request-0001"})
}

#[test]
fn requests_are_closed_and_restore_cannot_choose_an_overwrite_precondition() {
    let delete = body();
    assert!(parse(delete.to_string().as_bytes()).is_ok());
    for key in [
        "actor",
        "scope_hash",
        "scope_epoch",
        "limits",
        "precondition",
        "content",
        "target_path",
    ] {
        let mut value = delete.clone();
        value[key] = true.into();
        assert!(parse(value.to_string().as_bytes()).is_err());
    }
    let mut restore = delete.clone();
    restore["kind"] = "restore_tombstone".into();
    restore["tombstone_id"] = "b".repeat(64).into();
    assert!(parse(restore.to_string().as_bytes()).is_err());
    restore.as_object_mut().unwrap().remove("expected_revision");
    assert!(matches!(
        parse(restore.to_string().as_bytes()).unwrap().kind,
        Kind::Restore(_)
    ));
    for text in [
        "[]".to_owned(),
        "null".to_owned(),
        format!("{delete}{delete}"),
        delete
            .to_string()
            .replacen("\"kind\":", "\"kind\":\"delete\",\"kind\":", 1),
    ] {
        assert!(parse(text.as_bytes()).is_err());
    }
    assert!(parse(&[0xff]).is_err());
    assert!(parse(&vec![b' '; JSON_LIMIT + 1]).is_err());
}

#[test]
fn revision_path_key_and_paging_budgets_are_explicit() {
    for (key, value) in [
        ("expected_revision", "A".repeat(64)),
        ("idempotency_key", "short".into()),
        ("idempotency_key", "x".repeat(129)),
        ("idempotency_key", "control\nkey1234567".into()),
        ("path", "../outside.md".into()),
    ] {
        let mut input = body();
        input[key] = value.into();
        assert!(parse(input.to_string().as_bytes()).is_err());
    }
    let vault = body()["vault_uuid"].clone();
    for limit in [0, 65] {
        let query: PageQuery =
            serde_json::from_value(json!({"vault_uuid":vault,"limit":limit})).unwrap();
        assert!(!query.valid());
    }
    for cursor in [
        "".to_owned(),
        "x".repeat(101),
        "x\n".into(),
        "非ASCII".into(),
    ] {
        let query: PageQuery =
            serde_json::from_value(json!({"vault_uuid":vault,"cursor":cursor})).unwrap();
        assert!(!query.valid());
    }
}
